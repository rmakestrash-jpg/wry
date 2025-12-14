// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
  cell::{Cell, UnsafeCell},
  path::PathBuf,
  rc::Rc,
};

use gtk4::{
  gdk::{ContentFormats, DragAction, FileList},
  gio,
  glib,
  prelude::*,
  DropTargetAsync,
};
use webkit6::WebView;

use crate::DragDropEvent;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
enum DragControllerState {
  Entered,
  Leaving,
  Left,
}

struct DragDropController {
  paths: UnsafeCell<Option<Vec<PathBuf>>>,
  state: Cell<DragControllerState>,
  position: Cell<(i32, i32)>,
  handler: Box<dyn Fn(DragDropEvent) -> bool>,
}

impl DragDropController {
  fn new(handler: Box<dyn Fn(DragDropEvent) -> bool>) -> Self {
    Self {
      handler,
      paths: UnsafeCell::new(None),
      state: Cell::new(DragControllerState::Left),
      position: Cell::new((0, 0)),
    }
  }

  fn store_paths(&self, paths: Vec<PathBuf>) {
    unsafe { *self.paths.get() = Some(paths) };
  }

  fn take_paths(&self) -> Option<Vec<PathBuf>> {
    unsafe { &mut *self.paths.get() }.take()
  }

  fn store_position(&self, position: (i32, i32)) {
    self.position.replace(position);
  }

  fn enter(&self) {
    self.state.set(DragControllerState::Entered);
  }

  fn leaving(&self) {
    self.state.set(DragControllerState::Leaving);
  }

  fn leave(&self) {
    self.state.set(DragControllerState::Left);
  }

  fn state(&self) -> DragControllerState {
    self.state.get()
  }

  fn call(&self, event: DragDropEvent) -> bool {
    (self.handler)(event)
  }
}

pub(crate) fn connect_drag_event(webview: &WebView, handler: Box<dyn Fn(DragDropEvent) -> bool>) {
  let controller = Rc::new(DragDropController::new(handler));

  // GTK4: Use DropTargetAsync which provides Drop object directly in signals
  // FileList (requires v4_6) allows multiple files to be dropped at once
  let formats = ContentFormats::for_type(FileList::static_type());
  let drop_target = DropTargetAsync::new(Some(formats), DragAction::COPY);

  // Handle drag enter - Drop object is provided directly
  {
    let controller = controller.clone();
    drop_target.connect_drag_enter(move |_target, drop, x, y| {
      controller.store_position((x as i32, y as i32));

      // Read files asynchronously from the Drop object
      let controller = controller.clone();
      drop.read_value_async(
        FileList::static_type(),
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |result| {
          if let Ok(value) = result {
            if let Ok(file_list) = value.get::<FileList>() {
              let paths: Vec<PathBuf> = file_list.files().iter().map(path_buf_from_file).collect();
              if !paths.is_empty() {
                controller.enter();
                controller.call(DragDropEvent::Enter {
                  paths: paths.clone(),
                  position: controller.position.get(),
                });
                controller.store_paths(paths);
              }
            }
          }
        },
      );

      DragAction::COPY
    });
  }

  // Handle drag motion (hover)
  {
    let controller = controller.clone();
    drop_target.connect_drag_motion(move |_target, _drop, x, y| {
      if controller.state() == DragControllerState::Entered {
        controller.call(DragDropEvent::Over {
          position: (x as i32, y as i32),
        });
      } else {
        controller.store_position((x as i32, y as i32));
      }
      DragAction::COPY
    });
  }

  // Handle drop
  {
    let controller = controller.clone();
    drop_target.connect_drop(move |_target, drop, x, y| {
      let controller = controller.clone();
      let position = (x as i32, y as i32);

      // Read the files from the drop
      drop.read_value_async(
        FileList::static_type(),
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |result| {
          if let Ok(value) = result {
            if let Ok(file_list) = value.get::<FileList>() {
              let paths: Vec<PathBuf> = file_list.files().iter().map(path_buf_from_file).collect();
              if !paths.is_empty() {
                controller.leave();
                controller.call(DragDropEvent::Drop { paths, position });
                return;
              }
            }
          }

          // Fall back to stored paths if async read failed
          if let Some(paths) = controller.take_paths() {
            controller.leave();
            controller.call(DragDropEvent::Drop { paths, position });
          }
        },
      );

      true // Accept the drop
    });
  }

  // Handle drag leave
  {
    drop_target.connect_drag_leave(move |_target, _drop| {
      if controller.state() != DragControllerState::Left {
        controller.leaving();
        let controller = controller.clone();
        glib::idle_add_local_once(move || {
          if controller.state() == DragControllerState::Leaving {
            controller.leave();
            controller.call(DragDropEvent::Leave);
          }
        });
      }
    });
  }

  webview.add_controller(drop_target);
}

fn path_buf_from_file(file: &gio::File) -> PathBuf {
  if let Some(path) = file.path() {
    path
  } else {
    // uri() returns GString directly in gio 0.21+
    let uri = file.uri();
    path_buf_from_uri(uri.as_str())
  }
}

fn path_buf_from_uri(uri: &str) -> PathBuf {
  let path = uri.strip_prefix("file://").unwrap_or(uri);
  let path = percent_encoding::percent_decode(path.as_bytes())
    .decode_utf8_lossy()
    .to_string();
  PathBuf::from(path)
}
