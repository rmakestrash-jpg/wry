// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Minimal GTK4 + webkit6 test example.
//!
//! This example tests the GTK4 migration directly without depending on tao.
//! Run with: cargo run --example gtk4_simple --features os-webview

use gtk4::{glib, prelude::*, Application, ApplicationWindow, Box as GtkBox, Orientation};
use wry::{WebViewBuilder, WebViewBuilderExtUnix};

const APP_ID: &str = "org.wry.Gtk4Simple";

fn main() -> glib::ExitCode {
  let app = Application::builder().application_id(APP_ID).build();

  app.connect_activate(build_ui);
  app.run()
}

fn build_ui(app: &Application) {
  // Create main window
  let window = ApplicationWindow::builder()
    .application(app)
    .title("WRY GTK4 Test")
    .default_width(1024)
    .default_height(768)
    .build();

  // Create a vertical box to hold the webview
  let vbox = GtkBox::new(Orientation::Vertical, 0);
  vbox.set_vexpand(true);
  vbox.set_hexpand(true);
  window.set_child(Some(&vbox));

  // Create webview using the GTK4 path
  let webview = WebViewBuilder::new()
    .with_url("https://tauri.app")
    .with_devtools(true)
    .build_gtk(&vbox);

  match webview {
    Ok(_wv) => {
      println!("WebView created successfully!");
      println!("Loading https://tauri.app");
    }
    Err(e) => {
      eprintln!("Failed to create WebView: {e}");
      // Create a label showing the error
      let label = gtk4::Label::new(Some(&format!("Error: {e}")));
      vbox.append(&label);
    }
  }

  window.present();
}
