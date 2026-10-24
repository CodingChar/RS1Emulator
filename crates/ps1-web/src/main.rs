use wasm_bindgen::prelude::*;
use wasm_bindgen::Clamped;
use wasm_bindgen::JsCast;
use web_sys::{window, CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
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

    let f = Rc::new(RefCell::new(None as Option<Closure<dyn FnMut()>>));
    let g = f.clone();

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let _ = ps1.run_frame();

        // The size follows the game's video mode, so read it every frame.
        let (fb_w, fb_h) = (ps1.framebuffer_width(), ps1.framebuffer_height());
        if canvas.width() != fb_w || canvas.height() != fb_h {
            canvas.set_width(fb_w);
            canvas.set_height(fb_h);
        }
        let fb = ps1.framebuffer();
        if let Ok(image) = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&fb[..]), fb_w, fb_h) {
            let _ = ctx.put_image_data(&image, 0.0, 0.0);
        }

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
