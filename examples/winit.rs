// Note: This example is incompatible with Linux since GTK4 does not support
// embedding into raw X11 window handles. On Linux, use WebViewBuilderExtUnix::new_gtk()
// with a GTK4 container instead. See examples/gtk4_simple.rs for the recommended approach.
#[cfg(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
))]
fn main() {
  eprintln!("This example is incompatible with Linux.");
  eprintln!("GTK4 does not support embedding into raw X11 window handles.");
  eprintln!("See examples/gtk4_simple.rs for the recommended approach on Linux.");
}

#[cfg(not(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
)))]
use dpi::{LogicalPosition, LogicalSize};
#[cfg(not(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
)))]
use winit::{
  application::ApplicationHandler,
  event::WindowEvent,
  event_loop::{ActiveEventLoop, EventLoop},
  window::{Window, WindowId},
};
#[cfg(not(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
)))]
use wry::{Rect, WebViewBuilder};

#[cfg(not(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
)))]
#[derive(Default)]
struct State {
  window: Option<Window>,
  webview: Option<wry::WebView>,
}

#[cfg(not(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
)))]
impl ApplicationHandler for State {
  fn resumed(&mut self, event_loop: &ActiveEventLoop) {
    let mut attributes = Window::default_attributes();
    attributes.inner_size = Some(LogicalSize::new(800, 800).into());
    let window = event_loop.create_window(attributes).unwrap();

    let webview = WebViewBuilder::new()
      .with_url("https://tauri.app")
      .build_as_child(&window)
      .unwrap();

    self.window = Some(window);
    self.webview = Some(webview);
  }

  fn window_event(
    &mut self,
    _event_loop: &ActiveEventLoop,
    _window_id: WindowId,
    event: WindowEvent,
  ) {
    match event {
      WindowEvent::Resized(size) => {
        let window = self.window.as_ref().unwrap();
        let webview = self.webview.as_ref().unwrap();

        let size = size.to_logical::<u32>(window.scale_factor());
        webview
          .set_bounds(Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: LogicalSize::new(size.width, size.height).into(),
          })
          .unwrap();
      }
      WindowEvent::CloseRequested => {
        std::process::exit(0);
      }
      _ => {}
    }
  }

  fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
}

#[cfg(not(any(
  target_os = "linux",
  target_os = "dragonfly",
  target_os = "freebsd",
  target_os = "netbsd",
  target_os = "openbsd"
)))]
fn main() -> wry::Result<()> {
  let event_loop = EventLoop::new().unwrap();
  let mut state = State::default();
  event_loop.run_app(&mut state).unwrap();

  Ok(())
}
