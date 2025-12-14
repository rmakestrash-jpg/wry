// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use tao::{
  event::{Event, WindowEvent},
  event_loop::{ControlFlow, EventLoop},
  window::WindowBuilder,
};
use wry::WebViewBuilder;

#[cfg(not(any(
  target_os = "windows",
  target_os = "macos",
  target_os = "ios",
  target_os = "android"
)))]
use wry::WebViewBuilderExtUnix;

fn main() -> wry::Result<()> {
  let event_loop = EventLoop::new();
  let window = WindowBuilder::new().build(&event_loop).unwrap();

  // On Windows/macOS, use bounds for positioning multiple webviews
  #[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
  ))]
  let (webview, webview2, webview3, webview4) = {
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::Rect;

    let size = window.inner_size().to_logical::<u32>(window.scale_factor());

    let webview = WebViewBuilder::new()
      .with_bounds(Rect {
        position: LogicalPosition::new(0, 0).into(),
        size: LogicalSize::new(size.width / 2, size.height / 2).into(),
      })
      .with_url("https://tauri.app")
      .build(&window)?;

    let webview2 = WebViewBuilder::new()
      .with_bounds(Rect {
        position: LogicalPosition::new(size.width / 2, 0).into(),
        size: LogicalSize::new(size.width / 2, size.height / 2).into(),
      })
      .with_url("https://github.com/tauri-apps/wry")
      .build(&window)?;

    let webview3 = WebViewBuilder::new()
      .with_bounds(Rect {
        position: LogicalPosition::new(0, size.height / 2).into(),
        size: LogicalSize::new(size.width / 2, size.height / 2).into(),
      })
      .with_url("https://twitter.com/TauriApps")
      .build(&window)?;

    let webview4 = WebViewBuilder::new()
      .with_bounds(Rect {
        position: LogicalPosition::new(size.width / 2, size.height / 2).into(),
        size: LogicalSize::new(size.width / 2, size.height / 2).into(),
      })
      .with_url("https://google.com")
      .build(&window)?;

    (webview, webview2, webview3, webview4)
  };

  // On Linux, use a Grid layout for proper 2x2 quadrant sizing
  #[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
  )))]
  let (webview, webview2, webview3, webview4) = {
    use gtk4::prelude::*;
    use tao::platform::unix::WindowExtUnix;

    let vbox = window.default_vbox().unwrap();

    // Create a 2x2 grid
    let grid = gtk4::Grid::new();
    grid.set_row_homogeneous(true);
    grid.set_column_homogeneous(true);
    grid.set_hexpand(true);
    grid.set_vexpand(true);
    vbox.append(&grid);

    // Create a Box for each quadrant (wry builds into Box containers)
    let box1 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let box2 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let box3 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let box4 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Attach boxes to grid cells
    grid.attach(&box1, 0, 0, 1, 1); // top-left
    grid.attach(&box2, 1, 0, 1, 1); // top-right
    grid.attach(&box3, 0, 1, 1, 1); // bottom-left
    grid.attach(&box4, 1, 1, 1, 1); // bottom-right

    let webview = WebViewBuilder::new()
      .with_url("https://tauri.app")
      .build_gtk(&box1)?;

    let webview2 = WebViewBuilder::new()
      .with_url("https://github.com/tauri-apps/wry")
      .build_gtk(&box2)?;

    let webview3 = WebViewBuilder::new()
      .with_url("https://twitter.com/TauriApps")
      .build_gtk(&box3)?;

    let webview4 = WebViewBuilder::new()
      .with_url("https://google.com")
      .build_gtk(&box4)?;

    (webview, webview2, webview3, webview4)
  };

  event_loop.run(move |event, _, control_flow| {
    *control_flow = ControlFlow::Wait;

    match event {
      #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
      ))]
      Event::WindowEvent {
        event: WindowEvent::Resized(size),
        ..
      } => {
        use wry::dpi::{LogicalPosition, LogicalSize};
        use wry::Rect;

        let size = size.to_logical::<u32>(window.scale_factor());
        webview
          .set_bounds(Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: LogicalSize::new(size.width / 2, size.height / 2).into(),
          })
          .unwrap();
        webview2
          .set_bounds(Rect {
            position: LogicalPosition::new(size.width / 2, 0).into(),
            size: LogicalSize::new(size.width / 2, size.height / 2).into(),
          })
          .unwrap();
        webview3
          .set_bounds(Rect {
            position: LogicalPosition::new(0, size.height / 2).into(),
            size: LogicalSize::new(size.width / 2, size.height / 2).into(),
          })
          .unwrap();
        webview4
          .set_bounds(Rect {
            position: LogicalPosition::new(size.width / 2, size.height / 2).into(),
            size: LogicalSize::new(size.width / 2, size.height / 2).into(),
          })
          .unwrap();
      }
      Event::WindowEvent {
        event: WindowEvent::CloseRequested,
        ..
      } => *control_flow = ControlFlow::Exit,
      _ => {
        // Keep webviews alive
        let _ = (&webview, &webview2, &webview3, &webview4);
      }
    }
  });
}
