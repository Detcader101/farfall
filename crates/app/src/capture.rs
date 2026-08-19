//! Frame capture: the app screenshots itself (SPEC §8).
//!
//! The renderer already owns every pixel it draws, so reading them back needs
//! no screen-recording permission, no window picker, and no cooperation from
//! the desktop. That makes captures reliable in a way OS-level screenshots are
//! not — and it is the same texture-to-buffer readback the golden-image tests
//! need, so this is test infrastructure that happens to be useful by hand.
//!
//! The scene target is captured, not the swapchain: that is the world without
//! the HUD painted over it, which is what you want to compare frames against.

use std::path::{Path, PathBuf};

/// wgpu requires each row of a texture-to-buffer copy to start on a 256-byte
/// boundary, so the buffer is wider than the image and the padding is stripped
/// after mapping.
const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

pub struct Capture {
    buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
    width: u32,
    height: u32,
    path: PathBuf,
}

impl Capture {
    /// Record a copy of `texture` into a staging buffer. Call between encoding
    /// the scene pass and submitting; finish with [`Capture::save`] after the
    /// queue has been polled to completion.
    pub fn record(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        path: impl AsRef<Path>,
    ) -> Self {
        let width = texture.width();
        let height = texture.height();
        let unpadded = width * 4;
        let padded_bytes_per_row = unpadded.div_ceil(ROW_ALIGN) * ROW_ALIGN;

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture staging"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        Self {
            buffer,
            padded_bytes_per_row,
            width,
            height,
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Map the staging buffer and write a PNG. Blocks; only ever called on an
    /// explicit key press, never in the steady frame loop.
    pub fn save(self, device: &wgpu::Device, bgra: bool) -> Result<PathBuf, String> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("poll failed: {e:?}"))?;
        rx.recv()
            .map_err(|e| format!("map channel closed: {e}"))?
            .map_err(|e| format!("buffer map failed: {e:?}"))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| format!("mapped range unavailable: {e:?}"))?;
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in 0..self.height {
            let start = (row * self.padded_bytes_per_row) as usize;
            let end = start + (self.width * 4) as usize;
            // The surface format is typically BGRA; PNG wants RGBA.
            for px in mapped[start..end].chunks_exact(4) {
                if bgra {
                    pixels.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                } else {
                    pixels.extend_from_slice(px);
                }
            }
        }
        drop(mapped);
        self.buffer.unmap();

        let file = std::fs::File::create(&self.path)
            .map_err(|e| format!("cannot create {}: {e}", self.path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .map_err(|e| format!("png header: {e}"))?
            .write_image_data(&pixels)
            .map_err(|e| format!("png data: {e}"))?;
        Ok(self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The padded row stride must be a multiple of the alignment and never
    /// narrower than the image, or the copy silently corrupts every row.
    #[test]
    fn row_padding_satisfies_the_alignment_rule() {
        for width in [1u32, 63, 64, 65, 100, 640, 1919, 2880] {
            let unpadded = width * 4;
            let padded = unpadded.div_ceil(ROW_ALIGN) * ROW_ALIGN;
            assert_eq!(padded % ROW_ALIGN, 0, "width {width} misaligned");
            assert!(padded >= unpadded, "width {width} padded too narrow");
            assert!(
                padded - unpadded < ROW_ALIGN,
                "width {width} padded further than necessary"
            );
        }
    }
}
