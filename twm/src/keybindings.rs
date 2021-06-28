use crate::{keyboardhook::{self, InputEvent}, config::Config, event::Event, popup::Popup, system, system::api, AppState};
use key::Key;
use keybinding::Keybinding;
use log::{debug, error, info};
use modifier::Modifier;
use std::collections::HashMap;
use num_traits::FromPrimitive;
use parking_lot::Mutex;
use std::{
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::channel,
    sync::mpsc::Receiver,
    sync::mpsc::Sender,
    sync::Arc,
    thread,
    time::Duration,
};

pub mod key;
pub mod keybinding;
pub mod modifier;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum KeybindingsMessage {
    UpdateKeybindings,
}

fn get_keybindings(state: &AppState) -> HashMap<i32, Keybinding> {
    let mut kbs = HashMap::new();
    for kb in &state.config.keybindings {
        kbs.insert(kb.get_id(), kb.clone());
    }
    kbs
}

pub fn listen(state_arc: Arc<Mutex<AppState>>) -> Sender<KeybindingsMessage> {
    let (tx, rx) = channel::<KeybindingsMessage>();
    let event_tx = state_arc.lock().event_channel.sender.clone();

    std::thread::spawn(move || {
        let mut kbs = get_keybindings(&state_arc.lock());
        let hook = keyboardhook::start();
        while let Ok(ev) = hook.recv() {
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    KeybindingsMessage::UpdateKeybindings => {
                        kbs = get_keybindings(&state_arc.lock());
                    }
                }
            }

            if let InputEvent::KeyDown { key_code, shift, ctrl, win, lalt, ralt } = ev {
                let mut modifiers = Modifier::empty();

                if ctrl {
                    modifiers |= Modifier::CONTROL;
                }
                if win {
                    modifiers |= Modifier::WIN;
                }
                if shift {
                    modifiers |= Modifier::SHIFT;
                }
                if lalt {
                    modifiers |= Modifier::LALT;
                }
                if ralt {
                    modifiers |= Modifier::RALT;
                }

                let id = (key_code as u32 + modifiers.bits() * 1000) as i32;

                if let Some(kb) = kbs.get(&id) {
                    hook.block(true);
                    event_tx.send(Event::Keybinding(kb.clone()));
                    continue;
                }
            }

            hook.block(false);
        }
    });

    tx
}
