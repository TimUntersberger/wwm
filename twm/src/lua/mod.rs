use std::{str::FromStr, sync::Arc};

use chrono::Local;
use mlua::{Error as LuaError, FromLua, Function, Lua, Table, ToLua, Value};
use parking_lot::Mutex;
use regex::Regex;

use log::{info, error, warn};

use crate::{
    bar::component::Component, config::bar_config::BarComponentsConfig, config::rule::Rule,
    config::workspace_setting::WorkspaceSetting, config::Config, direction::Direction,
    event::Event, get_config_path, keybindings::keybinding::Keybinding,
    keybindings::keybinding::KeybindingKind, split_direction::SplitDirection, system,
    system::DisplayId, system::WindowId, AppState,
get_runtime_path, popup::Popup};

mod conversions;
mod runtime;

pub use runtime::{get_err_msg, LuaRuntime};

/// This is macro is necessary, because if you use the default way of type checking input
/// (specifying the type in the function declaration) the error is different than how it would look
/// like when you check the type at runtime.
///
/// This macro basically does the same thing, but throws a
/// runtime error on the lua side instead of a CallbackError on the rust side.
macro_rules! validate {
    ($lua: tt, $x: ident : $type: ty) => {
        match <$type>::from_lua($x.clone(), $lua) {
            Ok(x) => Ok(x),
            Err(_) => {
                let msg = format!("Expected `{}` to be of type `{}` (found `{}`)", stringify!($x), stringify!($type), $x.type_name());
                Err(LuaError::RuntimeError(msg))
            }
        }
    };
    ($lua: tt, $x: ident : $type: ty, $x_name: tt) => {
        match <$type>::from_lua($x.clone(), $lua) {
            Ok(x) => Ok(x),
            Err(_) => {
                let msg = format!("Expected `{}` to be of type `{}` (found `{}`)", $x_name, stringify!($type), $x.type_name());
                Err(LuaError::RuntimeError(msg))
            }
        }
    };
    ($lua: tt, $x: ident : $type: ty, $x_name: tt, $ty_name: tt) => {
        match <$type>::from_lua($x.clone(), $lua) {
            Ok(x) => Ok(x),
            Err(_) => {
                let msg = format!("Expected `{}` to be of type `{}` (found `{}`)", $x_name, $ty_name, $x.type_name());
                Err(LuaError::RuntimeError(msg))
            }
        }
    };
    ($lua: ident, { $($x: ident : $type: ty),* }) => {
        $(let $x = validate!($lua, $x: $type)?;)*
    }
}

macro_rules! validate_tbl_prop {
    ($lua: tt, $tbl: tt, $x: ident, $t: ty) => {
        let $x = match $tbl.get::<_, Value>(stringify!($x)) {
            Err(_) => {
                return Err(LuaError::RuntimeError(format!(
                    "A component needs to have a {} field",
                    stringify!($x)
                )))
            }
            Ok(x) => x,
        };
        let $x = validate!($lua, $x: $t)?;
    };
}

fn comp_from_tbl(
    state_arc: Arc<Mutex<AppState>>,
    lua: &Lua,
    tbl: Table,
) -> mlua::Result<Component> {
    validate_tbl_prop!(lua, tbl, name, String);
    validate_tbl_prop!(lua, tbl, render, Function);
    validate_tbl_prop!(lua, tbl, on_click, Option<Function>);

    let id = LuaRuntime::add_callback(lua, render)?;
    let state = state_arc.clone();
    let mut comp = Component::new(&name, move |disp_id| {
        let rt = state.lock().lua_rt.clone();
        let res = rt.with_lua(|lua| {
            let cb = LuaRuntime::get_callback(&lua, id)?;
            cb.call(disp_id.0)
        });

        Ok(match res {
            Err(e) => {
                state.lock().emit_lua_rt_error(&crate::lua::get_err_msg(&e));
                vec![]
            }
            Ok(x) => x,
        })
    });

    if let Some(on_click_fn) = on_click {
        let id = LuaRuntime::add_callback(lua, on_click_fn)?;
        let state = state_arc.clone();
        comp.with_on_click(move |display_id, value, idx| {
            let rt = state.lock().lua_rt.clone();
            let res = rt.with_lua(|lua| {
                let cb = LuaRuntime::get_callback(&lua, id)?;
                cb.call::<_, ()>((display_id.0, value, idx))
            });

            if let Err(e) = res {
                state.lock().emit_lua_rt_error(&crate::lua::get_err_msg(&e));
            }

            Ok(())
        });
        comp.lua_on_click_id = Some(id);
    }

    comp.lua_render_id = Some(id);

    Ok(comp)
}

macro_rules! def_fn {
    ($lua: expr, $tbl: expr, $name: expr, $func: expr) => {
        $tbl.set($name, $lua.create_function($func)?)?;
    };
}

/// This is used to map a lua function to a rust function.
macro_rules! def_ffi_fn {
    ($state_arc: expr, $lua: expr, $tbl: expr, $name: expr, $func_name: ident, $a1:ident : $a1t: ty, $a2:ident : $a2t: ty) => {
        let state = $state_arc.clone();
        def_fn!($lua, $tbl, $name, move |lua, ($a1, $a2): (Value, Value)| {
            let $a1 = validate!(lua, $a1: $a1t)?;
            let $a2 = validate!(lua, $a2: $a2t)?;
            state
                .lock()
                .$func_name($a1, $a2)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        });
    };
    ($state_arc: expr, $lua: expr, $tbl: expr, $name: expr, $func_name: ident, $a1: ident : $a1t: tt) => {
        let state = $state_arc.clone();
        def_fn!($lua, $tbl, $name, move |lua, $a1: Value| {
            let $a1 = validate!(lua, $a1: $a1t)?;
            state
                .lock()
                .$func_name($a1)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        });
    };
    ($state_arc: expr, $lua: expr, $tbl: expr, $name: expr, $func_name: ident) => {
        let state = $state_arc.clone();
        def_fn!($lua, $tbl, $name, move |_, (): ()| {
            state
                .lock()
                .$func_name()
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        });
    };
}

fn config_to_lua<'a>(lua: &'a Lua, config: &Config) -> mlua::Result<Table<'a>> {
    let tbl = lua.create_table()?;
    let rules_tbl = lua.create_table()?;
    let workspaces_tbl = lua.create_table()?;
    let bar_tbl = lua.create_table()?;
    let bar_components_tbl = lua.create_table()?;

    macro_rules! map_prop {
        ($tbl: tt, $path: expr, $name: tt) => {
            $tbl.set(stringify!($name), $path.$name)?;
        };
        ($tbl: tt, $path: expr, $name: tt, true) => {
            $tbl.set(stringify!($name), $path.$name.clone())?;
        };
    }

    map_prop!(tbl, config, launch_on_startup);
    map_prop!(tbl, config, enable_hot_reloading);
    map_prop!(tbl, config, min_height);
    map_prop!(tbl, config, min_width);
    map_prop!(tbl, config, use_border);
    map_prop!(tbl, config, outer_gap);
    map_prop!(tbl, config, inner_gap);
    map_prop!(tbl, config, remove_title_bar);
    map_prop!(tbl, config, work_mode);
    map_prop!(tbl, config, light_theme);
    map_prop!(tbl, config, multi_monitor);
    map_prop!(tbl, config, remove_task_bar);
    map_prop!(tbl, config, display_app_bar);
    map_prop!(tbl, config, ignore_fullscreen_actions);
    map_prop!(tbl, config, allow_right_alt);

    map_prop!(bar_tbl, config.bar, color);
    map_prop!(bar_tbl, config.bar, height);
    map_prop!(bar_tbl, config.bar, font, true);
    map_prop!(bar_tbl, config.bar, font_size);

    map_prop!(bar_components_tbl, config.bar.components, left, true);
    map_prop!(bar_components_tbl, config.bar.components, center, true);
    map_prop!(bar_components_tbl, config.bar.components, right, true);

    for ws in &config.workspaces {
        let tbl = lua.create_table()?;

        tbl.set("monitor", ws.monitor)?;
        tbl.set("text", ws.text.clone())?;

        workspaces_tbl.set(ws.id, tbl)?;
    }

    for rule in &config.rules {
        let tbl = lua.create_table()?;

        tbl.set("chromium", rule.chromium)?;
        tbl.set("firefox", rule.firefox)?;
        tbl.set("has_custom_titlebar", rule.has_custom_titlebar)?;
        tbl.set("ignore", !rule.manage)?;
        tbl.set("workspace_id", rule.workspace_id)?;

        rules_tbl.set(rule.pattern.to_string(), tbl)?;
    }

    bar_tbl.set("components", bar_components_tbl)?;
    tbl.set("bar", bar_tbl)?;
    tbl.set("workspaces", workspaces_tbl)?;
    tbl.set("rules", rules_tbl)?;

    Ok(tbl)
}

fn ws_from_tbl(lua: &Lua, id: i32, tbl: Table) -> mlua::Result<WorkspaceSetting> {
    let mut ws = WorkspaceSetting::default();
    ws.id = id;

    for pair in tbl.pairs::<String, Value>() {
        if let Ok((key, val)) = pair {
            match key.as_str() {
                "text" => ws.text = FromLua::from_lua(val, lua)?,
                "monitor" => ws.monitor = FromLua::from_lua(val, lua)?,
                _ => {}
            }
        }
    }

    Ok(ws)
}

fn rule_from_tbl(lua: &Lua, raw_pat: String, tbl: Table) -> mlua::Result<Rule> {
    let mut rule = Rule::default();
    rule.pattern = Regex::new(&raw_pat).map_err(|e| LuaError::RuntimeError(e.to_string()))?;

    for pair in tbl.pairs::<String, Value>() {
        if let Ok((key, val)) = pair {
            match key.as_str() {
                "ignore" => rule.manage = !FromLua::from_lua(val, lua)?,
                "chromium" => rule.chromium = FromLua::from_lua(val, lua)?,
                "firefox" => rule.firefox = FromLua::from_lua(val, lua)?,
                "has_custom_titlebar" => rule.has_custom_titlebar = FromLua::from_lua(val, lua)?,
                "workspace_id" => rule.workspace_id = FromLua::from_lua(val, lua)?,
                _ => {}
            }
        }
    }

    Ok(rule)
}

fn components_from_tbl(
    state_arc: Arc<Mutex<AppState>>,
    lua: &Lua,
    tbl: Table,
) -> mlua::Result<BarComponentsConfig> {
    let mut components = BarComponentsConfig::new();

    if let Ok(tbl) = tbl.get::<_, Table>("left") {
        for res in tbl.sequence_values::<Table>() {
            if let Ok(comp_tbl) = res {
                components
                    .left
                    .push(comp_from_tbl(state_arc.clone(), lua, comp_tbl)?);
            }
        }
    }

    if let Ok(tbl) = tbl.get::<_, Table>("center") {
        for res in tbl.sequence_values::<Table>() {
            if let Ok(comp_tbl) = res {
                components
                    .center
                    .push(comp_from_tbl(state_arc.clone(), lua, comp_tbl)?);
            }
        }
    }

    if let Ok(tbl) = tbl.get::<_, Table>("right") {
        for res in tbl.sequence_values::<Table>() {
            if let Ok(comp_tbl) = res {
                components
                    .right
                    .push(comp_from_tbl(state_arc.clone(), lua, comp_tbl)?);
            }
        }
    }

    Ok(components)
}

fn setup_nog_global(state_arc: Arc<Mutex<AppState>>, rt: &LuaRuntime) {
    rt.with_lua(move |lua| {
        let nog_tbl = lua.create_table()?;
        let cb_tbl = lua.create_table()?;

        // This never gets cleaned up which could causes some performance problems after a long
        // time
        nog_tbl.set("__callbacks", cb_tbl)?;
        nog_tbl.set("__is_setup", true)?;
        nog_tbl.set("version", option_env!("NOG_VERSION").unwrap_or("DEV"))?;
        nog_tbl.set("runtime_path", get_runtime_path().to_str())?;
        nog_tbl.set("config_path", get_config_path().to_str())?;

        let state = state_arc.clone();
        nog_tbl.set("config", config_to_lua(lua, &state.lock().config)?)?;

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "__on_config_updated", move |lua,
                                                           (
            prefix,
            key,
            value,
        ): (
            Value,
            Value,
            Value
        )| {
            validate!(lua, { prefix: String, key: String });
            let parts = prefix.split('.').collect::<Vec<_>>();
            let state_arc = state.clone();
            let mut state = state_arc.lock();
            macro_rules! set_prop {
                (bar, $name: tt, $type: ty) => {
                    state.config.bar.$name = validate!(lua, value: $type)?
                };
                ($name: tt, $type: ty) => {
                    state.config.$name = validate!(lua, value: $type)?
                };
            }
            match parts.as_slice() {
                ["nog", "config"] => match key.as_str() {
                    "launch_on_startup" => set_prop!(launch_on_startup, bool),
                    "enable_hot_reloading" => set_prop!(enable_hot_reloading, bool),
                    "min_height" => set_prop!(min_height, i32),
                    "min_width" => set_prop!(min_width, i32),
                    "use_border" => set_prop!(use_border, bool),
                    "outer_gap" => set_prop!(outer_gap, i32),
                    "inner_gap" => set_prop!(inner_gap, i32),
                    "remove_title_bar" => set_prop!(remove_title_bar, bool),
                    "work_mode" => set_prop!(work_mode, bool),
                    "light_theme" => set_prop!(light_theme, bool),
                    "multi_monitor" => set_prop!(multi_monitor, bool),
                    "remove_task_bar" => set_prop!(remove_task_bar, bool),
                    "display_app_bar" => set_prop!(display_app_bar, bool),
                    "ignore_fullscreen_actions" => set_prop!(ignore_fullscreen_actions, bool),
                    "allow_right_alt" => set_prop!(allow_right_alt, bool),
                    "workspaces" => {
                        let tbl = validate!(lua, value: Table)?;
                        let mut workspaces = Vec::new();
                        for res in tbl.pairs::<i32, Table>() {
                            if let Ok((id, tbl)) = res {
                                workspaces.push(ws_from_tbl(lua, id, tbl)?);
                            }
                        }
                        state.config.workspaces = workspaces;
                    }
                    "rules" => {
                        let tbl = validate!(lua, value: Table)?;
                        let mut rules = Vec::new();
                        for res in tbl.pairs::<String, Table>() {
                            if let Ok((pat, tbl)) = res {
                                rules.push(rule_from_tbl(lua, pat, tbl)?);
                            }
                        }
                        state.config.rules = rules;
                    }
                    x => warn!("Unknown config key {}", x),
                },
                ["nog", "config", "bar"] => match key.as_str() {
                    "color" => set_prop!(bar, color, i32),
                    "height" => set_prop!(bar, height, i32),
                    "font" => set_prop!(bar, font, String),
                    "font_size" => set_prop!(bar, font_size, i32),
                    "components" => {
                        let tbl = validate!(lua, value: Table)?;
                        state.config.bar.components =
                            components_from_tbl(state_arc.clone(), lua, tbl)?;
                    }
                    x => warn!("Unknown config key {}", x),
                },
                x => unreachable!("Unsupported {:?}", x),
            }
            Ok(())
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "quit", move |_, (): ()| {
            let _ = state.lock().event_channel.sender.send(Event::Exit);
            Ok(())
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_keybindings", move |lua, (): ()| {
            Ok(state.lock().config.keybindings.clone())
        });

        def_fn!(lua, nog_tbl, "popup_close", move |_, (): ()| {
            crate::popup::close();

            Ok(())
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "popup_create", move |lua, settings: Value| {
            validate!(lua, { settings: Table });
            let mut popup = Popup::new();

            for res in settings.pairs::<String, Value>() {
                if let Ok((key, value)) = res {
                    popup = match key.as_str() {
                        "text" => popup.with_text(validate!(lua, value : Vec<String>, "text", "string[]")?),
                        "padding" => popup.with_padding(validate!(lua, value : i32)?),
                        _ => popup
                    }
                }
            }

            popup
                .create(state.clone())
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;

            Ok(())
        });

        def_fn!(lua, nog_tbl, "launch", move |lua, name: Value| {
            validate!(lua, { name: String });
            let name = name.replace("~", dirs::home_dir().unwrap().to_str().unwrap());
            system::api::launch_program(name)?;
            Ok(())
        });

        def_fn!(lua, nog_tbl, "fmt_datetime", move |lua, pat: Value| {
            validate!(lua, { pat: String });
            let text = Local::now().format(&pat).to_string();
            Ok(text)
        });

        let state = state_arc.clone();
        def_fn!(
            lua,
            nog_tbl,
            "get_active_ws_of_display",
            move |lua, display_id: Value| {
                validate!(lua, { display_id: i32 });
                let state_g = state.lock();
                let grids = state_g
                    .get_display_by_id(DisplayId(display_id))
                    .unwrap()
                    .get_active_grids();

                Ok(grids.iter().map(|g| g.id).collect::<Vec<_>>())
            }
        );

        let state = state_arc.clone();
        def_fn!(
            lua,
            nog_tbl,
            "get_focused_ws_of_display",
            move |lua, display_id: Value| {
                validate!(lua, { display_id: i32 });
                Ok(state.lock().get_focused_ws_of_display(display_id))
            }
        );

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "toggle_work_mode", move |_, (): ()| {
            AppState::toggle_work_mode(state.clone())
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "is_ws_focused", move |lua, ws_id: Value| {
            validate!(lua, { ws_id: i32 });
            Ok(state.lock().workspace_id == ws_id)
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_win_title", move |lua, win_id: Value| {
            validate!(lua, { win_id: i32 });
            let win_id = WindowId(win_id);
            Ok(state
                .lock()
                .find_grid_containing_window(win_id)
                .and_then(|g| g.get_window(win_id))
                .map(|w| w.get_title().unwrap_or_default()))
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_ws_info", move |lua, ws_id: Value| {
            validate!(lua, { ws_id: i32 });
            Ok(state.lock().get_grid_by_id(ws_id).map(|ws| {
                let tbl = lua.create_table().unwrap();
                tbl.set("id", ws.id);
                tbl.set("is_fullscreen", ws.is_fullscreened());
                tbl.set("is_empty", ws.is_empty());
                tbl.set("split_direction", ws.next_axis.to_string());
                let windows = ws.get_windows().iter().map(|w| w.id.0).collect::<Vec<_>>();
                tbl.set("windows", windows);
                tbl
            }))
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_current_display_id", move |_, (): ()| {
            Ok(state.lock().get_current_display().id.0)
        });

        def_fn!(lua, nog_tbl, "scale_color", move |lua,
                                                   (color, factor): (
            Value,
            Value
        )| {
            validate!(lua, {
                color: i32,
                factor: f64
            });

            Ok(crate::util::scale_color(color, factor))
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_current_win", move |_, (): ()| {
            let win_id = state
                .lock()
                .get_current_display()
                .get_focused_grid()
                .and_then(|g| g.get_focused_window())
                .map(|w| w.id.0);

            Ok(win_id)
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_focused_win_of_display", move |lua, display_id: Value| {
            validate!(lua, { display_id: i32 });
            Ok(state.lock().get_focused_win_of_display(display_id))
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_current_ws", move |_, (): ()| {
            let ws_id = state
                .lock()
                .get_current_display()
                .get_focused_grid()
                .map(|ws| ws.id);

            Ok(ws_id)
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "__unbind_batch", move |lua, kbs: Value| {
            validate!(lua, {
                kbs: Vec<Keybinding>
            });

            let mut state = state.lock();
            for s_kb in &kbs {
                if let Some(i) = state
                    .config
                    .keybindings
                    .iter()
                    .enumerate()
                    .find(|(_, kb)| kb.key == s_kb.key && kb.modifier == s_kb.modifier)
                    .map(|(i, _)| i)
                {
                    state.config.keybindings.remove(i);
                }
            }
            let tx = state.event_channel.sender.clone();
            drop(state);
            tx.send(Event::UpdateKeybindings);
            Ok(())
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "unbind", move |lua, key: Value| {
            validate!(lua, { key: String });
            let mut state_g = state.lock();
            // The dummy keybinding that is being searched for
            let s_kb = Keybinding::from_str(&key).unwrap();
            if let Some((i, kb)) = state_g
                .config
                .keybindings
                .iter()
                .enumerate()
                .find(|(_, kb)| kb.key == s_kb.key && kb.modifier == s_kb.modifier)
                .map(|(i, kb)| (i, kb.clone()))
            {
                state_g.config.keybindings.remove(i);
                let tx = state_g.event_channel.sender.clone();
                drop(state_g);
                tx.send(Event::UpdateKeybindings);
            }

            Ok(())
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "get_ws_text", move |lua, ws_id: Value| {
            validate!(lua, { ws_id: i32 });
            Ok(state.lock().get_ws_text(ws_id))
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "__bind_batch", move |lua, kbs: Value| {
            validate!(lua, {
                kbs: Vec<Keybinding>
            });

            let mut state = state.lock();
            for kb in &kbs {
                state.config.keybindings.push(kb.clone());
            }
            let tx = state.event_channel.sender.clone();
            drop(state);
            tx.send(Event::UpdateKeybindings);
            Ok(())
        });

        let state = state_arc.clone();
        def_fn!(lua, nog_tbl, "__bind", move |lua,
                                            (mode, key, id): (
            Value,
            Value,
            Value
        )| {
            validate!(lua, {
                mode: String,
                key: String,
                id: usize
            });
            let mut kb = Keybinding::from_str(&key).unwrap();
            kb.kind = match mode.as_str() {
                "g" => KeybindingKind::Global,
                "w" => KeybindingKind::Work,
                _ => KeybindingKind::Normal,
            };

            kb.callback_id = id;
            let mut state = state.lock();
            state.config.keybindings.push(kb.clone());
            let tx = state.event_channel.sender.clone();
            drop(state);
            tx.send(Event::UpdateKeybindings);
            Ok(())
        });

        let globals = lua.globals();
        globals.set("nog", nog_tbl)?;

        Ok(())
    })
    .unwrap();
}

fn load_window_functions(state_arc: Arc<Mutex<AppState>>, rt: &LuaRuntime) -> mlua::Result<()> {
    rt.with_lua(|lua| {
        let nog_tbl = lua.globals().get::<_, Table>("nog")?;

        /// A local version of the `def_ffi_fn` macro for ease of use.
        ///
        /// **Note**: Also prefixes the name with `win_`
        macro_rules! l_def_ffi_fn {
            ($name: expr, $($rest: tt)*) => {
                def_ffi_fn!(state_arc, lua, nog_tbl, format!("{}_{}", "win", $name), $($rest)*);
            };
        }

        l_def_ffi_fn!("minimize", minimize_window);
        l_def_ffi_fn!("toggle_floating", toggle_floating);
        l_def_ffi_fn!("ignore", ignore_window);
        l_def_ffi_fn!("close", close_window);
        l_def_ffi_fn!("move_to_ws", move_window_to_workspace, ws_id: i32);

        Ok(())
    })
}

fn load_plugin_functions(state_arc: Arc<Mutex<AppState>>, rt: &LuaRuntime) -> mlua::Result<()> {
    rt.with_lua(|lua| {
        let nog_tbl = lua.globals().get::<_, Table>("nog")?;

        /// A local version of the `def_ffi_fn` macro for ease of use.
        ///
        /// **Note**: Also prefixes the name with `plug_`
        macro_rules! l_def_ffi_fn {
            ($name: expr, $($rest: tt)*) => {
                def_ffi_fn!(state_arc, lua, nog_tbl, format!("{}_{}", "plug", $name), $($rest)*);
            };
        }

        l_def_ffi_fn!("install", install_plugin, name: String);
        l_def_ffi_fn!("update", update_plugins);
        l_def_ffi_fn!("uninstall", uninstall_plugin, name: String);
        l_def_ffi_fn!("list", get_plugins);

        Ok(())
    })
}

fn load_workspace_functions(state_arc: Arc<Mutex<AppState>>, rt: &LuaRuntime) -> mlua::Result<()> {
    rt.with_lua(|lua| {
        let nog_tbl = lua.globals().get::<_, Table>("nog")?;

        /// A local version of the `def_ffi_fn` macro for ease of use.
        ///
        /// **Note**: Also prefixes the name with `ws_`
        macro_rules! l_def_ffi_fn {
            ($name: expr, $($rest: tt)*) => {
                def_ffi_fn!(state_arc, lua, nog_tbl, format!("{}_{}", "ws", $name), $($rest)*);
            };
        }

        l_def_ffi_fn!("toggle_fullscreen", toggle_fullscreen);
        l_def_ffi_fn!("reset_row", reset_row);
        l_def_ffi_fn!("reset_col", reset_column);
        l_def_ffi_fn!(
            "move_to_monitor",
            move_workspace_to_monitor,
            monitor_id: i32
        );
        l_def_ffi_fn!("replace", move_workspace_to_workspace, ws_id: i32);
        l_def_ffi_fn!("change", emit_change_workspace, ws_id: i32);
        l_def_ffi_fn!("move_in", move_in, direction: Direction);
        l_def_ffi_fn!("move_out", move_out, direction: Direction);
        l_def_ffi_fn!("swap_columns_and_rows", swap_columns_and_rows);
        l_def_ffi_fn!("focus", focus, direction: Direction);
        l_def_ffi_fn!("resize", resize, direction: Direction, amount: i32);
        l_def_ffi_fn!("swap", swap, direction: Direction);
        l_def_ffi_fn!(
            "set_split_direction",
            set_split_direction,
            direction: SplitDirection
        );

        Ok(())
    })
}

pub fn setup_lua_rt(state_arc: Arc<Mutex<AppState>>) {
    let rt = state_arc.lock().lua_rt.clone();

    setup_nog_global(state_arc.clone(), &rt);

    rt.with_lua(|lua| {
        let state = state_arc.lock();
        let package_tbl = lua.globals().get::<_, Table>("package")?;
        let mut path = package_tbl.get::<_, String>("path")?;
        let mut cpath = package_tbl.get::<_, String>("cpath")?;

        for plug_path in state.get_plugins().unwrap() {
            path = format!("{}\\lua\\?.lua;{}", plug_path, path);
        }

        path = format!("{}\\config\\?.lua;{}", get_config_path().to_str().unwrap(), path);

        #[cfg(debug_assertions)]
        let dll_path = {
            let mut path = std::env::current_exe().unwrap();
            path.pop();
            path.pop();
            path.pop();
            path.push("twm");
            path.push("runtime");
            path.push("dll");
            path
        };
        #[cfg(not(debug_assertions))]
        let dll_path = {
            let mut path = get_runtime_path();
            path.push("dll");
            path
        };

        info!("DLL path: {:?}", dll_path);

        cpath = format!("{}\\?.dll;{}", dll_path.to_str().unwrap(), cpath);

        package_tbl.set("path", path)?;
        package_tbl.set("cpath", cpath)?;

        Ok(())
    }).unwrap();

    let mut path = get_runtime_path();
    path.push("lua");
    path.push("runtime.lua");

    rt.run_file(path);

    load_window_functions(state_arc.clone(), &rt).unwrap();
    load_workspace_functions(state_arc.clone(), &rt).unwrap();
    load_plugin_functions(state_arc.clone(), &rt).unwrap();
}
