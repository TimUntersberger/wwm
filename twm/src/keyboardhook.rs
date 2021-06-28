use winapi::shared::minwindef::LRESULT;
use winapi::shared::windef::HHOOK;
use winapi::um::winuser::{
    CallNextHookEx, DispatchMessageW, PeekMessageW, UnhookWindowsHookEx, SetWindowsHookExW, TranslateMessage,
    KBDLLHOOKSTRUCT, MSG, PM_REMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use std::sync::mpsc::{channel, Receiver, RecvError, Sender};
use std::time::Duration;

const TIMEOUT_DURATION: Duration = Duration::from_millis(100);
static mut CTRL: bool = false;
static mut SHIFT: bool = false;
static mut WIN: bool = false;
static mut LALT: bool = false;
static mut RALT: bool = false;
/// (input event emitter, control flow receiver, block receiver)
///
/// The block receiver only handles whether the keyboardhook should block the event from passing to other
/// applications.
///
/// Where `true` means the event should be blocked.
static mut HOOK_STATE: Option<(Sender<InputEvent>, Receiver<ControlFlow>, Receiver<bool>)> = None;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ControlFlow {
    Exit,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum InputEvent {
    KeyDown {
        key_code: usize,
        lalt: bool,
        ralt: bool,
        shift: bool,
        win: bool,
        ctrl: bool,
    },
    KeyUp {
        key_code: usize,
        lalt: bool,
        ralt: bool,
        shift: bool,
        win: bool,
        ctrl: bool,
    },
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum KbdHookEvent {
    KeyDown,
    KeyUp,
    SysKeyUp,
    SysKeyDown,
}

impl KbdHookEvent {
    pub fn from_usize(input: usize) -> Option<Self> {
        match input as u32 {
            WM_KEYDOWN => Some(Self::KeyDown),
            WM_KEYUP => Some(Self::KeyUp),
            WM_SYSKEYUP => Some(Self::SysKeyUp),
            WM_SYSKEYDOWN => Some(Self::SysKeyDown),
            _ => None,
        }
    }
}

pub struct KeyboardHook {
    /// input event receiver
    ie_rx: Receiver<InputEvent>,
    // control flow sender
    cf_tx: Sender<ControlFlow>,
    // block sender
    block_tx: Sender<bool>,
}

impl KeyboardHook {
    pub fn recv(&self) -> Result<InputEvent, RecvError> {
        self.ie_rx.recv()
    }

    pub fn exit(&self) {
        self.cf_tx.send(ControlFlow::Exit);
    }

    pub fn block(&self, value: bool) {
        self.block_tx.send(value);
    }
}

pub fn start() -> KeyboardHook {
    let (ie_sender, ie_receiver) = channel::<InputEvent>();
    let (cf_sender, cf_receiver) = channel::<ControlFlow>();
    let (block_sender, block_receiver) = channel::<bool>();

    let hook = KeyboardHook {
        ie_rx: ie_receiver,
        cf_tx: cf_sender,
        block_tx: block_sender,
    };

    unsafe {
        HOOK_STATE = Some((ie_sender, cf_receiver, block_receiver));
        std::thread::spawn(|| {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_cb), std::ptr::null_mut(), 0);
            let (_, cf_receiver, _) = HOOK_STATE.as_ref().unwrap();
            loop {
                let mut msg = MSG::default();
                if PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                if let Ok(cf) = cf_receiver.try_recv() {
                    match cf {
                        ControlFlow::Exit => {
                            HOOK_STATE = None;
                            UnhookWindowsHookEx(hook);
                            break;
                        }
                    }
                }
            }
        });
    }

    hook
}

unsafe extern "system" fn hook_cb(ncode: i32, wparam: usize, lparam: isize) -> LRESULT {
    if ncode >= 0 {
        let (ie_tx, _, block_rx) = HOOK_STATE.as_ref().unwrap();

        let kbdhook = lparam as *mut KBDLLHOOKSTRUCT;
        let key = (*kbdhook).vkCode;
        let event = KbdHookEvent::from_usize(wparam).unwrap();
        let mut block = false;

        match event {
            KbdHookEvent::KeyDown | KbdHookEvent::SysKeyDown => {
                match key {
                    162 => CTRL = true,
                    160 | 161 => SHIFT = true,
                    91 => WIN = true,
                    164 => LALT = true,
                    165 => RALT = true,
                    key => {
                        ie_tx
                            .send(InputEvent::KeyDown {
                                key_code: key as usize,
                                ctrl: CTRL,
                                shift: SHIFT,
                                win: WIN,
                                lalt: LALT,
                                ralt: RALT,
                            })
                            .unwrap();
                        block = true;
                    }
                };
            }
            KbdHookEvent::KeyUp | KbdHookEvent::SysKeyUp => {
                match key {
                    162 => CTRL = false,
                    160 | 161 => SHIFT = false,
                    91 => WIN = false,
                    164 => LALT = false,
                    165 => RALT = false,
                    _key => {
                        ie_tx
                            .send(InputEvent::KeyUp {
                                key_code: key as usize,
                                ctrl: CTRL,
                                shift: SHIFT,
                                win: WIN,
                                lalt: LALT,
                                ralt: RALT,
                            })
                            .unwrap();
                        block = true;
                    }
                };
            }
        }

        if block {
            if let Ok(block) = block_rx.recv_timeout(TIMEOUT_DURATION) {
                if block {
                    return 1;
                }
            }
        }

    }
    CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}
