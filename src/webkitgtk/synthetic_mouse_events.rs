use std::{cell::RefCell, rc::Rc};

use gtk4::{
  gdk::ModifierType,
  prelude::*,
  GestureClick,
};
use webkit6::prelude::*;
use webkit6::WebView;

pub fn setup(webview: &WebView) {
  let bf_state = BackForwardState(Rc::new(RefCell::new(0)));

  // GTK4: Use GestureClick instead of connect_button_press_event
  let gesture = GestureClick::new();
  gesture.set_button(0); // Listen for all buttons

  let bf_state_c = bf_state.clone();
  let webview_weak = webview.downgrade();
  gesture.connect_pressed(move |gesture, n_press, x, y| {
    let button = gesture.current_button();
    let Some(webview) = webview_weak.upgrade() else {
      return;
    };

    match button {
      // back button
      8 => {
        bf_state_c.set(BACK);
        let js = create_js_mouse_event(button, x, y, true, &bf_state_c, gesture, n_press);
        webview.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
      }
      // forward button
      9 => {
        bf_state_c.set(FORWARD);
        let js = create_js_mouse_event(button, x, y, true, &bf_state_c, gesture, n_press);
        webview.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
      }
      _ => {}
    }
  });

  let bf_state_c = bf_state.clone();
  let webview_weak = webview.downgrade();
  gesture.connect_released(move |gesture, n_press, x, y| {
    let button = gesture.current_button();
    let Some(webview) = webview_weak.upgrade() else {
      return;
    };

    match button {
      // back button
      8 => {
        bf_state_c.remove(BACK);
        let js = create_js_mouse_event(button, x, y, false, &bf_state_c, gesture, n_press);
        webview.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
      }
      // forward button
      9 => {
        bf_state_c.remove(FORWARD);
        let js = create_js_mouse_event(button, x, y, false, &bf_state_c, gesture, n_press);
        webview.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
      }
      _ => {}
    }
  });

  webview.add_controller(gesture);
}

fn create_js_mouse_event(
  button: u32,
  x: f64,
  y: f64,
  pressed: bool,
  state: &BackForwardState,
  gesture: &GestureClick,
  n_press: i32,
) -> String {
  let event_name = if pressed { "mousedown" } else { "mouseup" };
  // js equivalent https://developer.mozilla.org/en-US/docs/Web/API/MouseEvent/button
  let js_button = if button == 8 { 3 } else { 4 };
  let (x, y) = (x as i32, y as i32);

  // Get modifier state from the current event
  let modifiers_state = gesture
    .current_event()
    .map(|e| e.modifier_state())
    .unwrap_or(ModifierType::empty());

  let mut buttons = 0;
  // left button
  if modifiers_state.contains(ModifierType::BUTTON1_MASK) {
    buttons += 1;
  }
  // right button
  if modifiers_state.contains(ModifierType::BUTTON3_MASK) {
    buttons += 2;
  }
  // middle button
  if modifiers_state.contains(ModifierType::BUTTON2_MASK) {
    buttons += 4;
  }
  // back button
  if state.has(BACK) {
    buttons += 8;
  }
  // forward button
  if state.has(FORWARD) {
    buttons += 16;
  }

  // Click count from GTK4 GestureClick (1=single, 2=double, 3=triple)
  let detail = n_press;

  format!(
    r#"(() => {{
        const el = document.elementFromPoint({x},{y});
        const ev = new MouseEvent('{event_name}', {{
          view: window,
          button: {js_button},
          buttons: {buttons},
          x: {x},
          y: {y},
          bubbles: true,
          detail: {detail},
          cancelBubble: false,
          cancelable: true,
          clientX: {x},
          clientY: {y},
          composed: true,
          layerX: {x},
          layerY: {y},
          pageX: {x},
          pageY: {y},
          screenX: window.screenX + {x},
          screenY: window.screenY + {y},
          ctrlKey: {ctrl_key},
          metaKey: {meta_key},
          shiftKey: {shift_key},
          altKey: {alt_key},
        }});
        el.dispatchEvent(ev)
        if (!ev.defaultPrevented && "{event_name}" === "mouseup") {{
          if (ev.button === 3) {{
            window.history.back();
          }}
          if (ev.button === 4) {{
            window.history.forward();
          }}
        }}
      }})()"#,
    event_name = event_name,
    x = x,
    y = y,
    detail = detail,
    ctrl_key = modifiers_state.contains(ModifierType::CONTROL_MASK),
    alt_key = modifiers_state.contains(ModifierType::ALT_MASK),
    shift_key = modifiers_state.contains(ModifierType::SHIFT_MASK),
    meta_key = modifiers_state.contains(ModifierType::SUPER_MASK),
    js_button = js_button,
    buttons = buttons,
  )
}

// Internal modifiers to track whether BACK/FORWARD buttons are pressed
const BACK: u8 = 0b01;
const FORWARD: u8 = 0b10;

/// A single u8 that stores whether [BACK] and [FORWARD] are pressed or not
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BackForwardState(Rc<RefCell<u8>>);

impl BackForwardState {
  fn set(&self, button: u8) {
    *self.0.borrow_mut() |= button
  }

  fn remove(&self, button: u8) {
    *self.0.borrow_mut() &= !button
  }

  fn has(&self, button: u8) -> bool {
    let state = *self.0.borrow();
    state & !button != state
  }
}
