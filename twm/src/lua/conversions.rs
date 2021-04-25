use mlua::{Error as LuaError, FromLua, Function, Table, ToLua, Value};

use crate::{
    bar::component::Component, config::workspace_setting::WorkspaceSetting,
    keybindings::keybinding::Keybinding, split_direction::SplitDirection,
};
use crate::{bar::component::ComponentText, direction::Direction, system::SystemError};
use std::str::FromStr;

use super::LuaRuntime;

impl From<SystemError> for LuaError {
    fn from(e: SystemError) -> Self {
        LuaError::RuntimeError(e.to_string())
    }
}

impl FromLua<'_> for Direction {
    fn from_lua(lua_value: mlua::Value<'_>, lua: &'_ mlua::Lua) -> mlua::Result<Self> {
        let raw_direction = String::from_lua(lua_value, lua)?;

        Ok(Direction::from_str(&raw_direction).unwrap_or(Direction::Right))
    }
}

impl FromLua<'_> for SplitDirection {
    fn from_lua(lua_value: mlua::Value<'_>, lua: &'_ mlua::Lua) -> mlua::Result<Self> {
        let raw_direction = String::from_lua(lua_value, lua)?;

        Ok(SplitDirection::from_str(&raw_direction).unwrap_or(SplitDirection::Horizontal))
    }
}

impl ToLua<'_> for Keybinding {
    fn to_lua(self, lua: &'_ mlua::Lua) -> mlua::Result<Value<'_>> {
        let tbl = lua.create_table()?;
        tbl.set("mode", self.kind.to_short_string())?;
        tbl.set("key", self.as_key_combo())?;

        tbl.to_lua(lua)
    }
}

impl ToLua<'_> for Component {
    fn to_lua(self, lua: &'_ mlua::Lua) -> mlua::Result<Value<'_>> {
        let tbl = lua.create_table()?;

        let render_id = self.lua_render_id.unwrap();
        let render_cb = LuaRuntime::get_callback(lua, render_id)?;

        tbl.set("name", self.name.clone())?;
        tbl.set("render", render_cb)?;

        if let Some(id) = self.lua_on_click_id {
            let cb = LuaRuntime::get_callback(lua, id)?;
            tbl.set("on_click", cb);
        }

        tbl.to_lua(lua)
    }
}

impl FromLua<'_> for ComponentText {
    fn from_lua(lua_value: mlua::Value<'_>, lua: &'_ mlua::Lua) -> mlua::Result<Self> {
        let text = match lua_value {
            Value::Nil => ComponentText::new().with_display_text("nil".into()),
            Value::Boolean(x) => ComponentText::new().with_display_text(x.to_string()),
            Value::Integer(x) => ComponentText::new().with_display_text(x.to_string()),
            Value::Number(x) => ComponentText::new().with_display_text(x.to_string()),
            Value::String(x) => {
                ComponentText::new().with_display_text(x.to_str().unwrap().to_string())
            }
            Value::Table(tbl) => {
                let mut comp = ComponentText::new();
                for res in tbl.pairs::<String, Value>() {
                    if let Ok((key, val)) = res {
                        match key.as_ref() {
                            "text" => {
                                comp = comp.with_display_text(String::from_lua(val, lua)?);
                            }
                            "value" => {
                                comp = comp.with_value(i32::from_lua(val, lua)?);
                            }
                            "fg" => {
                                comp = comp.with_foreground_color(i32::from_lua(val, lua)?);
                            }
                            "bg" => {
                                comp = comp.with_background_color(i32::from_lua(val, lua)?);
                            }
                            _ => {}
                        }
                    }
                }
                comp
            }
            _ => ComponentText::new(),
        };

        Ok(text)
    }
}
