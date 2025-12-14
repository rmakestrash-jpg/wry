// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use dpi::LogicalSize;
use gtk4::{
  gdk::{self},
  gio::Cancellable,
  glib,
  prelude::*,
};
use http::Request;
use raw_window_handle::HasWindowHandle;
#[cfg(any(debug_assertions, feature = "devtools"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
  collections::HashMap,
  rc::Rc,
  sync::{Arc, Mutex},
};
use webkit6::prelude::*;
use webkit6::{
  AutoplayPolicy, LoadEvent, NavigationAction, NavigationPolicyDecision,
  NetworkProxyMode, NetworkProxySettings, PolicyDecisionType,
  URIRequest, UserContentInjectedFrames,
  UserContentManager, UserScript, UserScriptInjectionTime,
  WebView, WebsitePolicies,
};
use webkit6::ffi::{
  webkit_get_major_version, webkit_get_micro_version, webkit_get_minor_version,
  webkit_policy_decision_ignore, webkit_policy_decision_use,
};

pub use web_context::WebContextImpl;

use crate::{
  proxy::ProxyConfig, web_context::WebContext, Error, NewWindowFeatures, NewWindowOpener,
  NewWindowResponse, PageLoadEvent, Rect, Result, WebViewAttributes, RGBA,
};

use self::web_context::WebContextExt;

const WEBVIEW_ID: &str = "webview_id";

/// Connect to the WebView "create" signal with proper Option<Widget> support.
///
/// The webkit6-rs bindings incorrectly require a non-optional Widget return type,
/// but the C API allows returning NULL to deny popup creation. This function
/// provides the correct behavior by using raw signal connection.
///
/// See: https://webkitgtk.org/reference/webkit2gtk/stable/signal.WebView.create.html
fn connect_create_with_nullable_return<F>(webview: &WebView, f: F) -> gtk4::glib::SignalHandlerId
where
  F: Fn(&WebView, &mut NavigationAction) -> Option<gtk4::Widget> + 'static,
{
  use gtk4::glib::{
    signal::connect_raw,
    translate::{from_glib_borrow, from_glib_none, ToGlibPtr},
  };

  unsafe extern "C" fn create_trampoline<
    F: Fn(&WebView, &mut NavigationAction) -> Option<gtk4::Widget> + 'static,
  >(
    this: *mut webkit6::ffi::WebKitWebView,
    navigation_action: *mut webkit6::ffi::WebKitNavigationAction,
    f: gtk4::glib::ffi::gpointer,
  ) -> *mut gtk4::ffi::GtkWidget {
    let f: &F = &*(f as *const F);
    // webkit6 NavigationAction methods like request() require &mut self
    // Use from_glib_none to get an owned copy we can mutate
    let mut action: NavigationAction = from_glib_none(navigation_action);
    match f(&from_glib_borrow(this), &mut action) {
      Some(widget) => widget.to_glib_full(),
      None => std::ptr::null_mut(),
    }
  }

  unsafe {
    let f: Box<F> = Box::new(f);
    connect_raw(
      webview.as_ptr() as *mut _,
      c"create".as_ptr() as *const _,
      Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
        create_trampoline::<F> as *const (),
      )),
      Box::into_raw(f),
    )
  }
}

mod drag_drop;
mod synthetic_mouse_events;
mod web_context;

pub(crate) struct InnerWebView {
  id: String,
  pub webview: WebView,
  #[cfg(any(debug_assertions, feature = "devtools"))]
  is_inspector_open: Arc<AtomicBool>,
  pending_scripts: Arc<Mutex<Option<Vec<String>>>>,
  is_in_fixed_parent: bool,
}

impl Drop for InnerWebView {
  fn drop(&mut self) {
    // GTK4: Use unparent() instead of destroy()
    // The widget will be cleaned up when all references are dropped
    self.webview.unparent();
  }
}

impl InnerWebView {
  /// Create a new webview from a raw window handle.
  ///
  /// **Note:** GTK4 does not support embedding into raw X11 window handles.
  /// Use [`new_gtk`](Self::new_gtk) with a GTK container instead.
  pub fn new<W: HasWindowHandle>(
    _window: &W,
    _attributes: WebViewAttributes,
    _pl_attrs: super::PlatformSpecificWebViewAttributes,
  ) -> Result<Self> {
    #[cfg(feature = "x11")]
    {
      Err(Error::Gtk4X11EmbeddingUnsupported)
    }
    #[cfg(not(feature = "x11"))]
    {
      Err(Error::UnsupportedWindowHandle)
    }
  }

  /// Create a new webview as a child of a raw window handle.
  ///
  /// **Note:** GTK4 does not support embedding into raw X11 window handles.
  /// Use [`new_gtk`](Self::new_gtk) with a GTK container instead.
  pub fn new_as_child<W: HasWindowHandle>(
    _parent: &W,
    _attributes: WebViewAttributes,
    _pl_attrs: super::PlatformSpecificWebViewAttributes,
  ) -> Result<Self> {
    #[cfg(feature = "x11")]
    {
      Err(Error::Gtk4X11EmbeddingUnsupported)
    }
    #[cfg(not(feature = "x11"))]
    {
      Err(Error::UnsupportedWindowHandle)
    }
  }

  pub fn new_gtk<W>(
    container: &W,
    mut attributes: WebViewAttributes,
    pl_attrs: super::PlatformSpecificWebViewAttributes,
  ) -> Result<Self>
  where
    W: IsA<gtk4::Widget>,
  {
    // default_context allows us to create a scoped context on-demand
    let mut default_context;
    let web_context = if attributes.incognito {
      default_context = WebContext::new_ephemeral();
      &mut default_context
    } else {
      match attributes.context.take() {
        Some(w) => w,
        None => {
          default_context = Default::default();
          &mut default_context
        }
      }
    };
    if let Some(proxy_setting) = &attributes.proxy_config {
      let proxy_uri = match proxy_setting {
        ProxyConfig::Http(endpoint) => format!("http://{}:{}", endpoint.host, endpoint.port),
        ProxyConfig::Socks5(endpoint) => {
          format!("socks5://{}:{}", endpoint.host, endpoint.port)
        }
      };
      // webkit6: Proxy settings moved from WebsiteDataManager to NetworkSession
      let settings = NetworkProxySettings::new(Some(proxy_uri.as_str()), &[]);
      web_context
        .network_session()
        .set_proxy_settings(NetworkProxyMode::Custom, Some(&settings));
    }

    // Extension loading
    if let Some(extension_path) = &pl_attrs.extension_path {
      web_context.os.set_web_extensions_directory(extension_path);
    }

    let webview = Self::create_webview(web_context, &attributes, &pl_attrs);

    // Transparent
    if attributes.transparent {
      webview.set_background_color(&gdk::RGBA::new(0., 0., 0., 0.));
    } else {
      // background color
      if let Some((red, green, blue, alpha)) = attributes.background_color {
        webview.set_background_color(&gdk::RGBA::new(
          red as _, green as _, blue as _, alpha as _,
        ));
      }
    }

    // Webview Settings
    Self::set_webview_settings(&webview, &attributes);

    // Webview handlers
    Self::attach_handlers(&webview, web_context, &mut attributes);

    // IPC handler
    Self::attach_ipc_handler(webview.clone(), &mut attributes);

    // Drag drop handler
    if let Some(drag_drop_handler) = attributes.drag_drop_handler.take() {
      drag_drop::connect_drag_event(&webview, drag_drop_handler);
    }

    web_context.register_automation(webview.clone());

    let is_in_fixed_parent = Self::add_to_container(&webview, container, &attributes);

    #[cfg(any(debug_assertions, feature = "devtools"))]
    let is_inspector_open = Self::attach_inspector_handlers(&webview);

    let id = attributes
      .id
      .map(|id| id.to_string())
      .unwrap_or_else(|| (webview.as_ptr() as isize).to_string());
    unsafe { webview.set_data(WEBVIEW_ID, id.clone()) };

    let w = Self {
      id,
      webview,
      pending_scripts: Arc::new(Mutex::new(Some(Vec::new()))),
      is_in_fixed_parent,
      #[cfg(any(debug_assertions, feature = "devtools"))]
      is_inspector_open,
    };

    // Initialize message handler
    w.init("Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: function(x) { window.webkit.messageHandlers['ipc'].postMessage(x) } }) })", true)?;

    // Initialize scripts
    for init_script in attributes.initialization_scripts {
      w.init(&init_script.script, init_script.for_main_frame_only)?;
    }

    // Run pending webview.eval() scripts once webview loads.
    let pending_scripts = w.pending_scripts.clone();
    w.webview.connect_load_changed(move |webview, event| {
      if let LoadEvent::Committed = event {
        let mut pending_scripts_ = pending_scripts.lock().unwrap();
        if let Some(pending_scripts) = pending_scripts_.take() {
          let cancellable: Option<&Cancellable> = None;
          for script in pending_scripts {
            // webkit6: evaluate_javascript replaces run_javascript
            webview.evaluate_javascript(&script, None, None, cancellable, |_| ());
          }
        }
      }
    });

    // Custom protocols handler
    for (name, handler) in attributes.custom_protocols {
      web_context.register_uri_scheme(&name, handler)?;
    }

    // Navigation
    if let Some(url) = attributes.url {
      web_context.load_uri(w.webview.clone(), url, attributes.headers);
    } else if let Some(html) = attributes.html {
      w.webview.load_html(&html, None);
    }

    if attributes.visible {
      w.webview.set_visible(true);
    }

    if attributes.focused {
      w.webview.grab_focus();
    }

    Ok(w)
  }

  fn create_webview(
    web_context: &WebContext,
    attributes: &WebViewAttributes,
    pl_attrs: &super::PlatformSpecificWebViewAttributes,
  ) -> WebView {
    let mut builder = WebView::builder()
      .user_content_manager(&UserContentManager::new())
      .is_controlled_by_automation(web_context.allows_automation());

    if attributes.autoplay {
      builder = builder.website_policies(
        &WebsitePolicies::builder()
          .autoplay(AutoplayPolicy::Allow)
          .build(),
      );
    }

    if let Some(related_view) = &pl_attrs.related_view {
      builder = builder.related_view(related_view);
    } else {
      builder = builder.web_context(web_context.context());
    }

    builder.build()
  }

  fn set_webview_settings(webview: &WebView, attributes: &WebViewAttributes) {
    // Disable input preedit,fcitx input editor can anchor at edit cursor position
    if let Some(input_context) = webview.input_method_context() {
      input_context.set_enable_preedit(false);
    }

    // Note: set_use_system_appearance_for_scrollbars was removed in webkit6
    // GTK4 handles scrollbar theming automatically

    if let Some(settings) = WebViewExt::settings(webview) {
      // Enable webgl, webaudio, canvas features as default.
      settings.set_enable_webgl(true);
      settings.set_enable_webaudio(true);
      settings
        .set_enable_back_forward_navigation_gestures(attributes.back_forward_navigation_gestures);

      // Enable clipboard
      if attributes.clipboard {
        settings.set_javascript_can_access_clipboard(true);
      }

      // Enable App cache
      settings.set_enable_page_cache(true);

      // Set user agent
      settings.set_user_agent(attributes.user_agent.as_deref());

      // Devtools
      if attributes.devtools {
        settings.set_enable_developer_extras(true);
      }

      if attributes.javascript_disabled {
        settings.set_enable_javascript(false);
      }
    }
  }

  fn attach_handlers(
    webview: &WebView,
    web_context: &mut WebContext,
    attributes: &mut WebViewAttributes,
  ) {
    // window.close()
    // GTK4: Use try_close() instead of destroy()
    webview.connect_close(move |webview| {
      webview.try_close();
    });

    // Synthetic mouse events
    synthetic_mouse_events::setup(webview);

    // Document title changed handler
    if let Some(document_title_changed_handler) = attributes.document_title_changed_handler.take() {
      webview.connect_title_notify(move |webview| {
        let new_title = webview.title().map(|t| t.to_string()).unwrap_or_default();
        document_title_changed_handler(new_title)
      });
    }

    // Page load handler
    if let Some(on_page_load_handler) = attributes.on_page_load_handler.take() {
      webview.connect_load_changed(move |webview, load_event| match load_event {
        LoadEvent::Committed => {
          on_page_load_handler(PageLoadEvent::Started, webview.uri().unwrap().to_string());
        }
        LoadEvent::Finished => {
          on_page_load_handler(PageLoadEvent::Finished, webview.uri().unwrap().to_string());
        }
        _ => (),
      });
    }

    // window creation handler
    if let Some(new_window_req_handler) = attributes.new_window_req_handler.take() {
      let related_webviews = Rc::new(Mutex::new(HashMap::new()));
      // Use our custom signal connection that properly supports returning NULL
      // to deny popup creation. The webkit6-rs bindings don't support this.
      connect_create_with_nullable_return(webview, move |webview, action| {
        let url = match action
          .request()
          .and_then(|request| request.uri())
          .map(|uri| uri.as_str().to_string())
        {
          Some(url) => url,
          None => return None,
        };
        match new_window_req_handler(
          url.clone(),
          NewWindowFeatures {
            size: None,
            position: None,
            opener: NewWindowOpener {
              webview: webview.clone(),
            },
          },
        ) {
          NewWindowResponse::Allow => {
            let related_webviews = related_webviews.clone();
            let root = webview.root().unwrap();
            let window = root.dynamic_cast_ref::<gtk4::ApplicationWindow>().unwrap();
            let id = window.id();
            let app = window.application().unwrap();

            let window = gtk4::ApplicationWindow::builder()
              .application(&app)
              .title(&url)
              .build();
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            window.set_child(Some(&box_));

            let related_webviews_ = related_webviews.clone();
            window.connect_destroy(move |_| {
              related_webviews_.lock().unwrap().remove(&id);
            });

            window.set_visible(true);
            match Self::new_gtk(
              &box_,
              WebViewAttributes {
                ..Default::default()
              },
              super::PlatformSpecificWebViewAttributes {
                related_view: Some(webview.clone()),
                ..Default::default()
              },
            ) {
              Ok(webview) => {
                let widget = webview.webview.upcast_ref::<gtk4::Widget>().clone();
                related_webviews.lock().unwrap().insert(id, webview);
                Some(widget)
              }
              Err(_) => None,
            }
          }
          NewWindowResponse::Create { webview } => Some(webview.upcast::<gtk4::Widget>()),
          NewWindowResponse::Deny => None,
        }
      });
    }

    // Navigation handler
    if let Some(navigation_handler) = attributes.navigation_handler.take() {
      webview.connect_decide_policy(move |_webview, policy_decision, policy_type| {
        let handler = match policy_type {
          PolicyDecisionType::NavigationAction => &navigation_handler,
          _ => return false,
        };

        if let Some(policy) = policy_decision.dynamic_cast_ref::<NavigationPolicyDecision>() {
          // webkit6: navigation_action() and request() require &mut self
          if let Some(mut nav_action) = policy.navigation_action() {
            if let Some(uri_req) = nav_action.request() {
              if let Some(uri) = uri_req.uri() {
                let allow = handler(uri.to_string());
                let pointer = policy_decision.as_ptr();
                unsafe {
                  if allow {
                    webkit_policy_decision_use(pointer)
                  } else {
                    webkit_policy_decision_ignore(pointer)
                  }
                }

                return true;
              }
            }
          }
        }

        false
      });
    }

    // Download handler
    if attributes.download_started_handler.is_some()
      || attributes.download_completed_handler.is_some()
    {
      web_context.register_download_handler(
        attributes.download_started_handler.take(),
        attributes.download_completed_handler.take(),
      )
    }
  }

  fn add_to_container<W>(webview: &WebView, container: &W, attributes: &WebViewAttributes) -> bool
  where
    W: IsA<gtk4::Widget>,
  {
    let mut is_in_fixed_parent = false;

    let container_type = container.type_().name();
    if container_type == "GtkBox" {
      // GTK4: use append() instead of pack_start()
      container
        .dynamic_cast_ref::<gtk4::Box>()
        .unwrap()
        .append(webview);
      // For expansion, set the widget properties
      webview.set_hexpand(true);
      webview.set_vexpand(true);
    } else if container_type == "GtkFixed" {
      let scale_factor = webview.scale_factor() as f64;
      let (width, height) = attributes
        .bounds
        .map(|b| b.size.to_logical::<i32>(scale_factor))
        .map(Into::into)
        .unwrap_or((1, 1));
      let (x, y) = attributes
        .bounds
        .map(|b| b.position.to_logical::<f64>(scale_factor))
        .map(Into::into)
        .unwrap_or((0.0, 0.0));

      webview.set_size_request(width, height);

      container
        .dynamic_cast_ref::<gtk4::Fixed>()
        .unwrap()
        .put(webview, x, y);

      is_in_fixed_parent = true;
    } else if container_type == "GtkOverlay" {
      container
        .dynamic_cast_ref::<gtk4::Overlay>()
        .unwrap()
        .set_child(Some(webview));
    } else {
      // For other widget types, try to set as child if it supports it
      // In GTK4, different widgets have different child management APIs
      if let Some(window) = container.dynamic_cast_ref::<gtk4::Window>() {
        window.set_child(Some(webview));
      } else {
        // Last resort: try setting parent directly
        webview.set_parent(container);
      }
    }

    is_in_fixed_parent
  }

  fn attach_ipc_handler(webview: WebView, attributes: &mut WebViewAttributes) {
    // Message handler
    let ipc_handler = attributes.ipc_handler.take();
    let manager = webview
      .user_content_manager()
      .expect("WebView does not have UserContentManager");

    // Connect before registering as recommended by the docs
    // webkit6: the signal provides &javascriptcore::Value directly, not a wrapper
    manager.connect_script_message_received(None, move |_m, js_value| {
      #[cfg(feature = "tracing")]
      let _span = tracing::info_span!(parent: None, "wry::ipc::handle").entered();

      if let Some(ipc_handler) = &ipc_handler {
        ipc_handler(
          Request::builder()
            .uri(webview.uri().unwrap().to_string())
            .body(js_value.to_string().to_string())
            .unwrap(),
        );
      }
    });

    // Register the handler we just connected
    // webkit6: register_script_message_handler takes (name, world_name)
    manager.register_script_message_handler("ipc", None);
  }

  #[cfg(any(debug_assertions, feature = "devtools"))]
  fn attach_inspector_handlers(webview: &WebView) -> Arc<AtomicBool> {
    let is_inspector_open = Arc::new(AtomicBool::default());
    if let Some(inspector) = webview.inspector() {
      let is_inspector_open_ = is_inspector_open.clone();
      inspector.connect_bring_to_front(move |_| {
        is_inspector_open_.store(true, Ordering::Relaxed);
        false
      });
      let is_inspector_open_ = is_inspector_open.clone();
      inspector.connect_closed(move |_| {
        is_inspector_open_.store(false, Ordering::Relaxed);
      });
    }
    is_inspector_open
  }

  pub fn id(&self) -> crate::WebViewId<'_> {
    &self.id
  }

  pub fn print(&self) -> Result<()> {
    let print = webkit6::PrintOperation::new(&self.webview);
    print.run_dialog(None::<&gtk4::Window>);
    Ok(())
  }

  pub fn url(&self) -> Result<String> {
    Ok(self.webview.uri().unwrap_or_default().to_string())
  }

  pub fn eval(
    &self,
    js: &str,
    callback: Option<impl FnOnce(String) + Send + 'static>,
  ) -> Result<()> {
    if let Some(pending_scripts) = &mut *self.pending_scripts.lock().unwrap() {
      pending_scripts.push(js.into());
    } else {
      let cancellable: Option<&Cancellable> = None;

      #[cfg(feature = "tracing")]
      let span = SendEnteredSpan(tracing::debug_span!("wry::eval").entered());

      // webkit6: evaluate_javascript replaces run_javascript
      // Result is now Result<javascriptcore6::Value, glib::Error>
      self.webview.evaluate_javascript(js, None, None, cancellable, |result| {
        #[cfg(feature = "tracing")]
        drop(span);

        if let Some(callback) = callback {
          let result = result
            .ok()
            .and_then(|value| value.to_json(0))
            .map(|s| s.to_string())
            .unwrap_or_default();

          callback(result);
        }
      });
    }

    Ok(())
  }

  fn init(&self, js: &str, for_main_only: bool) -> Result<()> {
    if let Some(manager) = self.webview.user_content_manager() {
      let script = UserScript::new(
        js,
        if for_main_only {
          UserContentInjectedFrames::TopFrame
        } else {
          UserContentInjectedFrames::AllFrames
        },
        UserScriptInjectionTime::Start,
        &[],
        &[],
      );
      manager.add_script(&script);
    } else {
      return Err(Error::InitScriptError);
    }
    Ok(())
  }

  #[cfg(any(debug_assertions, feature = "devtools"))]
  pub fn open_devtools(&self) {
    if let Some(inspector) = self.webview.inspector() {
      inspector.show();
      // `bring-to-front` is not received in this case
      self.is_inspector_open.store(true, Ordering::Relaxed);
    }
  }

  #[cfg(any(debug_assertions, feature = "devtools"))]
  pub fn close_devtools(&self) {
    if let Some(inspector) = self.webview.inspector() {
      inspector.close();
    }
  }

  #[cfg(any(debug_assertions, feature = "devtools"))]
  pub fn is_devtools_open(&self) -> bool {
    self.is_inspector_open.load(Ordering::Relaxed)
  }

  pub fn zoom(&self, scale_factor: f64) -> Result<()> {
    self.webview.set_zoom_level(scale_factor);
    Ok(())
  }

  pub fn set_background_color(&self, (red, green, blue, alpha): RGBA) -> Result<()> {
    self.webview.set_background_color(&gdk::RGBA::new(
      red as _, green as _, blue as _, alpha as _,
    ));
    Ok(())
  }

  pub fn load_url(&self, url: &str) -> Result<()> {
    self.webview.load_uri(url);
    Ok(())
  }

  pub fn load_url_with_headers(&self, url: &str, headers: http::HeaderMap) -> Result<()> {
    // webkit6: Use URIRequest::new() instead of builder pattern
    let req = URIRequest::new(url);

    if let Some(req_headers) = req.http_headers() {
      for (header, value) in headers.iter() {
        req_headers.append(
          header.to_string().as_str(),
          value.to_str().unwrap_or_default(),
        );
      }
    }

    self.webview.load_request(&req);

    Ok(())
  }

  pub fn load_html(&self, html: &str) -> Result<()> {
    self.webview.load_html(html, None);
    Ok(())
  }

  pub fn reload(&self) -> Result<()> {
    self.webview.reload();
    Ok(())
  }

  pub fn clear_all_browsing_data(&self) -> Result<()> {
    // webkit6: WebsiteDataManager accessed through NetworkSession, not WebContext
    if let Some(network_session) = self.webview.network_session() {
      if let Some(data_manager) = network_session.website_data_manager() {
        data_manager.clear(
          webkit6::WebsiteDataTypes::ALL,
          glib::TimeSpan::from_seconds(0),
          None::<&Cancellable>,
          |_| {},
        );
      }
    }

    Ok(())
  }

  pub fn bounds(&self) -> Result<Rect> {
    let mut bounds = Rect::default();

    // GTK4: Use width() and height() instead of allocated_size()
    let width = self.webview.width();
    let height = self.webview.height();
    bounds.size = LogicalSize::new(width, height).into();

    Ok(bounds)
  }

  pub fn set_bounds(&self, bounds: Rect) -> Result<()> {
    let scale_factor = self.webview.scale_factor() as f64;
    let (width, height): (i32, i32) = bounds.size.to_logical::<i32>(scale_factor).into();
    let (x, y): (i32, i32) = bounds.position.to_logical::<i32>(scale_factor).into();

    if self.is_in_fixed_parent {
      // GTK4: use set_size_request for sizing in Fixed containers
      self.webview.set_size_request(width, height);
      // Move the widget in the Fixed parent
      if let Some(parent) = self.webview.parent() {
        if let Some(fixed) = parent.dynamic_cast_ref::<gtk4::Fixed>() {
          fixed.move_(&self.webview, x as f64, y as f64);
        }
      }
    }

    Ok(())
  }

  pub fn set_visible(&self, visible: bool) -> Result<()> {
    self.webview.set_visible(visible);
    Ok(())
  }

  pub fn focus(&self) -> Result<()> {
    self.webview.grab_focus();
    Ok(())
  }

  pub fn focus_parent(&self) -> Result<()> {
    // In GTK4, focus is handled differently - use the root/toplevel
    if let Some(root) = self.webview.root() {
      if let Some(window) = root.dynamic_cast_ref::<gtk4::Window>() {
        window.present();
      }
    }

    Ok(())
  }

  fn cookie_from_soup_cookie(mut cookie: soup::Cookie) -> cookie::Cookie<'static> {
    let name = cookie.name().map(|n| n.to_string()).unwrap_or_default();
    let value = cookie.value().map(|n| n.to_string()).unwrap_or_default();

    let mut cookie_builder = cookie::CookieBuilder::new(name, value);

    if let Some(domain) = cookie.domain().map(|n| n.to_string()) {
      cookie_builder = cookie_builder.domain(domain);
    }

    if let Some(path) = cookie.path().map(|n| n.to_string()) {
      cookie_builder = cookie_builder.path(path);
    }

    let http_only = cookie.is_http_only();
    cookie_builder = cookie_builder.http_only(http_only);

    let secure = cookie.is_secure();
    cookie_builder = cookie_builder.secure(secure);

    let same_site = cookie.same_site_policy();
    let same_site = match same_site {
      soup::SameSitePolicy::Lax => cookie::SameSite::Lax,
      soup::SameSitePolicy::Strict => cookie::SameSite::Strict,
      soup::SameSitePolicy::None => cookie::SameSite::None,
      _ => cookie::SameSite::None,
    };
    cookie_builder = cookie_builder.same_site(same_site);

    let expires = cookie.expires();
    let expires = match expires {
      Some(datetime) => cookie::time::OffsetDateTime::from_unix_timestamp(datetime.to_unix())
        .ok()
        .map(cookie::Expiration::DateTime),
      None => Some(cookie::Expiration::Session),
    };
    if let Some(expires) = expires {
      cookie_builder = cookie_builder.expires(expires);
    }

    cookie_builder.build()
  }

  fn cookie_into_soup_cookie(cookie: &cookie::Cookie<'_>) -> soup::Cookie {
    let mut soup_cookie = soup::Cookie::new(
      cookie.name(),
      cookie.value(),
      cookie.domain().unwrap_or(""),
      cookie.path().unwrap_or(""),
      cookie
        .max_age()
        .map(|d| d.whole_seconds() as i32)
        .unwrap_or(-1),
    );

    if let Some(dt) = cookie.expires_datetime() {
      soup_cookie.set_expires(&glib::DateTime::from_unix_utc(dt.unix_timestamp()).unwrap());
    }

    if let Some(http_only) = cookie.http_only() {
      soup_cookie.set_http_only(http_only);
    }

    if let Some(same_site) = cookie.same_site() {
      soup_cookie.set_same_site_policy(match same_site {
        cookie::SameSite::Lax => soup::SameSitePolicy::Lax,
        cookie::SameSite::Strict => soup::SameSitePolicy::Strict,
        cookie::SameSite::None => soup::SameSitePolicy::None,
      });
    }

    if let Some(secure) = cookie.secure() {
      soup_cookie.set_secure(secure);
    }

    soup_cookie
  }

  pub fn cookies_for_url(&self, url: &str) -> Result<Vec<cookie::Cookie<'static>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    // webkit6: Access through NetworkSession instead of WebView directly
    if let Some(cookies_manager) = self
      .webview
      .network_session()
      .and_then(|s| s.cookie_manager())
    {
      cookies_manager.cookies(url, None::<&Cancellable>, move |cookies| {
        let cookies = cookies.map(|cookies| {
          cookies
            .into_iter()
            .map(Self::cookie_from_soup_cookie)
            .collect()
        });
        let _ = tx.send(cookies);
      })
    }

    let context = glib::MainContext::default();
    loop {
      context.iteration(true);

      if let Ok(response) = rx.try_recv() {
        return response.map_err(Into::into);
      }
    }
  }

  pub fn cookies(&self) -> Result<Vec<cookie::Cookie<'static>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    // webkit6: CookieManager accessed via network_session(), not website_data_manager()
    if let Some(cookies_manager) = self
      .webview
      .network_session()
      .and_then(|session| session.cookie_manager())
    {
      cookies_manager.all_cookies(None::<&Cancellable>, move |cookies| {
        let cookies = cookies.map(|cookies| {
          cookies
            .into_iter()
            .map(Self::cookie_from_soup_cookie)
            .collect()
        });
        let _ = tx.send(cookies);
      })
    }

    let context = glib::MainContext::default();
    loop {
      context.iteration(true);

      if let Ok(response) = rx.try_recv() {
        return response.map_err(Into::into);
      }
    }
  }

  pub fn set_cookie(&self, cookie: &cookie::Cookie<'_>) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    // webkit6: CookieManager accessed via network_session(), not website_data_manager()
    if let Some(cookies_manager) = self
      .webview
      .network_session()
      .and_then(|session| session.cookie_manager())
    {
      let soup_cookie = Self::cookie_into_soup_cookie(cookie);
      cookies_manager.add_cookie(&soup_cookie, None::<&Cancellable>, move |ret| {
        let _ = tx.send(ret);
      });
    }

    let context = glib::MainContext::default();
    loop {
      context.iteration(true);

      if let Ok(response) = rx.try_recv() {
        return response.map_err(Into::into);
      }
    }
  }

  pub fn delete_cookie(&self, cookie: &cookie::Cookie<'_>) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    // webkit6: CookieManager accessed via network_session(), not website_data_manager()
    if let Some(cookies_manager) = self
      .webview
      .network_session()
      .and_then(|session| session.cookie_manager())
    {
      let soup_cookie = Self::cookie_into_soup_cookie(cookie);
      cookies_manager.delete_cookie(&soup_cookie, None::<&Cancellable>, move |ret| {
        let _ = tx.send(ret);
      });
    }

    let context = glib::MainContext::default();
    loop {
      context.iteration(true);

      if let Ok(response) = rx.try_recv() {
        return response.map_err(Into::into);
      }
    }
  }

  pub fn reparent<W>(&self, container: &W) -> Result<()>
  where
    W: gtk4::prelude::IsA<gtk4::Widget>,
  {
    // GTK4: First unparent from current parent
    self.webview.unparent();

    let container_type = container.type_().name();
    if container_type == "GtkBox" {
      container
        .dynamic_cast_ref::<gtk4::Box>()
        .unwrap()
        .append(&self.webview);
      self.webview.set_hexpand(true);
      self.webview.set_vexpand(true);
    } else if container_type == "GtkFixed" {
      container
        .dynamic_cast_ref::<gtk4::Fixed>()
        .unwrap()
        .put(&self.webview, 0.0, 0.0);
    } else if container_type == "GtkOverlay" {
      container
        .dynamic_cast_ref::<gtk4::Overlay>()
        .unwrap()
        .set_child(Some(&self.webview));
    } else if let Some(window) = container.dynamic_cast_ref::<gtk4::Window>() {
      window.set_child(Some(&self.webview));
    } else {
      self.webview.set_parent(container);
    }

    Ok(())
  }
}

pub fn platform_webview_version() -> Result<String> {
  let (major, minor, patch) = unsafe {
    (
      webkit_get_major_version(),
      webkit_get_minor_version(),
      webkit_get_micro_version(),
    )
  };
  Ok(format!("{major}.{minor}.{patch}"))
}

// SAFETY: only use this when you are sure the span will be dropped on the same thread it was entered
#[cfg(feature = "tracing")]
struct SendEnteredSpan(tracing::span::EnteredSpan);

#[cfg(feature = "tracing")]
unsafe impl Send for SendEnteredSpan {}

// Note: webkit6 already provides CookieManager::all_cookies() natively,
// so the custom FFI that was here for webkit2gtk has been removed.
