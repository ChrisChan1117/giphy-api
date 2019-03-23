use std::rc::Rc;

use js_sys::{ArrayBuffer, Uint8Array};
use prost::Message;
use seed::prelude::*;
use wasm_bindgen::{JsValue, JsCast};
use web_sys::{BinaryType, MessageEvent, WebSocket};

use crate::{
    AppState,
    proto::api::{RequestFrame, ResponseFrame},
    state::{Model, ModelEvent},
};

const WS_URL: &str = "ws://127.0.0.1:8080/ws/";

/// A closure taking a message event.
pub type HandleMessage = Closure<(dyn FnMut(MessageEvent) + 'static)>;

/// A closure taking a JS value.
pub type HandleValue = Closure<(dyn FnMut(JsValue) + 'static)>;

/// An enumeration of the types of closures used here.
pub enum WSClosure {
    HandleM(HandleMessage),
    HandleV(HandleValue),
}

/// The subset of the app's data model related to networking.
#[derive(Clone, Default)]
pub struct NetworkState {
    pub connected: bool,
    pub socket: Option<WebSocket>, // A populated value here does not indicate a live connection.
    pub closures: Vec<Rc<WSClosure>>,
}

/// An enumeration of all network related events to be handled.
#[derive(Clone)]
pub enum NetworkEvent {
    Connected,
    Disconnected,
    NewSocket(WebSocket),
    NewClosure(Rc<WSClosure>),
    SendRequest(RequestFrame),
}

impl NetworkEvent {
    /// The reducer for this state model.
    pub fn reducer(event: NetworkEvent, mut model: &mut Model) -> Update<ModelEvent> {
        match event {
            NetworkEvent::Connected => {
                model.network.connected = true;
                Render.into()
            }
            NetworkEvent::Disconnected => {
                model.network.connected = false;
                model.network.socket = None;
                model.network.closures.clear();
                Render.into()
            }
            NetworkEvent::NewSocket(ws) => {
                model.network.socket = Some(ws);
                Render.into()
            }
            NetworkEvent::NewClosure(cb) => {
                model.network.closures.push(cb);
                Skip.into()
            }
            NetworkEvent::SendRequest(req) => {
                let ws = match model.network.socket.as_ref() {
                    Some(ws) => ws,
                    None => return Skip.into()
                };
                let mut buf = vec![];
                req.encode(&mut buf).unwrap(); // This will never fail.
                ws.send_with_u8_array(buf.as_mut_slice())
                    .expect("Expected to be able to send socket message."); // TODO: handle this error condition.
                model.input_text = "".into();
                model.msg_tx_cnt += 1;
                Render.into()
            }
        }
    }

    /// Emit a new event for adding a network closure.
    pub fn new_closure(state: AppState, cb: WSClosure) {
        state.update(ModelEvent::Network(NetworkEvent::NewClosure(Rc::new(cb))));
    }
}

pub fn open_ws(state: AppState) {
    let ws = WebSocket::new(WS_URL).unwrap(); // TODO: handle this.
    ws.set_binary_type(BinaryType::Arraybuffer);
    state.update(ModelEvent::Network(NetworkEvent::NewSocket(ws.clone())));

    // Build handler for when connections are first open.
    let on_open = build_on_open(state.clone());
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    NetworkEvent::new_closure(state.clone(), WSClosure::HandleV(on_open));

    // Build handler for when connections are closed.
    let on_close = build_on_close(state.clone());
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    NetworkEvent::new_closure(state.clone(), WSClosure::HandleV(on_close));

    // Build message handler.
    let on_message = build_on_message(state.clone());
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    NetworkEvent::new_closure(state.clone(), WSClosure::HandleM(on_message));

    // Build error handler.
    let on_error = build_on_close(state.clone());
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    NetworkEvent::new_closure(state.clone(), WSClosure::HandleV(on_error));
}

/// Generate a handler function for when a connection is open.
fn build_on_open(state: AppState) -> HandleValue {
    let handler = move |_| {
        state.update(ModelEvent::Network(NetworkEvent::Connected));
    };
    Closure::wrap(Box::new(handler) as Box<FnMut(JsValue)>)
}

/// Generate a handler function for when a connection is closed.
fn build_on_close(state: AppState) -> HandleValue {
    let handler = move |_| {
        state.update(ModelEvent::Network(NetworkEvent::Disconnected));
    };
    Closure::wrap(Box::new(handler) as Box<FnMut(JsValue)>)
}

/// Generate a handler function used for websocket connections.
fn build_on_message(state: AppState) -> HandleMessage {
    let handler = move |ev: MessageEvent| {
        // Extract the raw bytes of the message.
        let buf = match ev.data().dyn_into::<ArrayBuffer>() {
            Ok(buf) => {
                let u8buf = Uint8Array::new(&buf);
                let mut decode_buf = vec![0; u8buf.byte_length() as usize];
                u8buf.copy_to(&mut decode_buf);
                decode_buf
            }
            Err(_) => {
                log!("Received an unexpected message from the server which was not a raw byte array.");
                return;
            }
        };

        // Decode the received message to our expected protobuf message type.
        let frame = match ResponseFrame::decode(buf) {
            Ok(frame) => frame,
            Err(err) => {
                log!(format!("Failed to decode server message: {:?}", err));
                return;
            }
        };

        // Process the recived message in our state update system.
        log!(format!("Decoded message: {:?}", &frame));
        state.update(ModelEvent::ServerMsg(frame));
    };
    Closure::wrap(Box::new(handler) as Box<FnMut(MessageEvent)>)
}
