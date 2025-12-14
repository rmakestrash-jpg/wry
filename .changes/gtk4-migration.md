---
"wry": major
---

**Breaking (Linux):** Migrated from GTK3/webkit2gtk to GTK4/webkit6.

- Updated from `gtk` 0.18 to `gtk4` 0.10
- Updated from `webkit2gtk` 2.0.1 to `webkit6` 0.5
- Updated from `javascriptcore-rs` to `javascriptcore6`
- Updated from `soup2` to `soup3`
- X11 raw window handle embedding is no longer supported; use `WebViewBuilderExtUnix::new_gtk()` with GTK4 containers
- System dependency changed from `libwebkit2gtk-4.1-dev` to `libwebkitgtk-6.0-dev`
