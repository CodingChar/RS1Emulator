use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{window, HtmlCanvasElement, CanvasRenderingContext2d};
use ps1_wasm::{Ps1, init_panic_hook};
use std::cell::RefCell;
use std::rc::Rc;

#[wasm_bindgen(start)]
pub fn run() -> Result<(), JsValue> {
    init_panic_hook();

    let document = window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let canvas = document
        .get_element_by_id("emu-canvas")
        .ok_or_else(|| JsValue::from_str("canvas#emu-canvas not found"))?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("#emu-canvas is not a canvas"))?;

    let ctx = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no 2d context"))?
        .dyn_into::<CanvasRenderingContext2d>()
        .map_err(|_| JsValue::from_str("not a 2d context"))?;

    let mut ps1 = Ps1::new();

    // Native PS1 resolution.
    let fb_w = ps1.framebuffer_width();
    let fb_h = ps1.framebuffer_height();
    canvas.set_width(fb_w);
    canvas.set_height(fb_h);

    let image_data = ctx
        .create_image_data_with_sw_and_sh(fb_w as f64, fb_h as f64)
        .map_err(|e| JsValue::from_str(&format!("create_image_data failed: {:?}", e)))?;

    let f = Rc::new(RefCell::new(None as Option<Closure<dyn FnMut()>>));
    let g = f.clone();

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let _ = ps1.run_frame();
        let fb = ps1.framebuffer();
        let mut data = image_data.data();
        data.copy_from_slice(&fb);
        let _ = ctx.put_image_data(&image_data, 0.0, 0.0);

        let win = window().unwrap();
        let _ = win.request_animation_frame(
            f.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
        );
    }) as Box<dyn FnMut()>));

    let win = window().unwrap();
    let _ = win.request_animation_frame(
        g.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
    );

    Ok(())
}

fn main() {
    let _ = run();
}
