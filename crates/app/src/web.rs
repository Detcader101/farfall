//! The browser shell: the same [`App`] as native, driven by winit's web
//! backend on a canvas the page supplies, with the device negotiated
//! asynchronously and the settings in localStorage. VR (WebXR) is driven
//! from the page's own XR frame loop through [`xr_frame`]: the page owns
//! the session and the compositor, the module renders the stereo pair.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::{WindowAttributes, WindowId};

use crate::{App, PendingGpu};

pub const CANVAS_ID: &str = "farfall";

thread_local! {
    /// A device that arrived from the browser, waiting for the event loop.
    pub static PENDING: RefCell<Option<PendingGpu>> = const { RefCell::new(None) };
    /// The running app, shared with the page's XR frame loop.
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

pub fn canvas() -> Option<web_sys::HtmlCanvasElement> {
    let doc = web_sys::window()?.document()?;
    let el = doc.get_element_by_id(CANVAS_ID)?;
    el.dyn_into::<web_sys::HtmlCanvasElement>().ok()
}

pub fn with_canvas(attrs: WindowAttributes) -> WindowAttributes {
    attrs.with_canvas(canvas()).with_prevent_default(true)
}

pub fn storage_get(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(key)
        .ok()?
}

pub fn storage_set(key: &str, value: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(key, value);
    }
}

/// The web app: the native one behind a shared handle, so the page's XR
/// loop can reach it between winit's events.
struct WebApp(Rc<RefCell<App>>);

impl ApplicationHandler for WebApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.0.borrow_mut().resumed(event_loop);
    }
    fn device_event(&mut self, el: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.0.borrow_mut().device_event(el, id, event);
    }
    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // While the headset drives the frame, winit's redraws stand down
        // — once the renderer is up; the device's arrival still needs it.
        if matches!(event, WindowEvent::RedrawRequested) && XR_ACTIVE.with(|x| x.get()) {
            self.0.borrow_mut().pick_up_pending();
            return;
        }
        self.0.borrow_mut().window_event(el, id, event);
    }
}

thread_local! {
    static XR_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start the game on the page's canvas. Called by the page once the
/// player has clicked (so the browser lets the audio context run).
#[wasm_bindgen]
pub fn run() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    let event_loop = EventLoop::new().expect("event loop");
    let app = Rc::new(RefCell::new(App::default()));
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));
    event_loop.spawn_app(WebApp(app));
}

/// Is the renderer up yet? The page waits on this before offering VR.
#[wasm_bindgen]
pub fn ready() -> bool {
    APP.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|app| app.borrow().gpu.is_some())
    })
}

/// The headset took the frame loop.
#[wasm_bindgen]
pub fn xr_begin() {
    XR_ACTIVE.with(|x| x.set(true));
}

/// The headset gave the frame loop back.
#[wasm_bindgen]
pub fn xr_end() {
    XR_ACTIVE.with(|x| x.set(false));
    if let Some(app) = APP.with(|a| a.borrow().clone()) {
        let mut app = app.borrow_mut();
        if let Some(game) = app.game.as_mut() {
            game.vr = None;
        }
        if let Some(gpu) = app.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }
}

/// One VR frame. `views` holds, per eye, 11 floats: the eye's orientation
/// quaternion (x y z w) and position (x y z) in the seated reference
/// space, and the frustum's tangents (left right up down, all positive).
/// `width`/`height` is one eye's target in pixels; the canvas becomes the
/// pair side by side, left eye first. Returns false if not yet ready.
#[wasm_bindgen]
pub fn xr_frame(views: &[f32], width: u32, height: u32) -> bool {
    let Some(app) = APP.with(|a| a.borrow().clone()) else {
        return false;
    };
    let mut app = app.borrow_mut();
    app.pick_up_pending();
    let App { gpu, game, audio } = &mut *app;
    let (Some(gpu), Some(game)) = (gpu.as_mut(), game.as_mut()) else {
        return false;
    };
    if views.len() < 22 {
        return false;
    }
    let eye = |i: usize| -> crate::VrEye {
        let v = &views[i * 11..i * 11 + 11];
        crate::VrEye {
            head: glam::Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize(),
            pos: glam::Vec3::new(v[4], v[5], v[6]),
            tan: [v[7], v[8], v[9], v[10]],
        }
    };
    game.vr = Some(crate::VrView {
        eyes: [eye(0), eye(1)],
    });
    let (w, h) = (width.max(1), height.max(1));
    if gpu.config.width != w * 2 || gpu.config.height != h {
        gpu.config.width = w * 2;
        gpu.config.height = h;
        gpu.surface.configure(&gpu.device, &gpu.config);
    }
    crate::redraw(gpu, game, audio.as_ref(), None);
    true
}
