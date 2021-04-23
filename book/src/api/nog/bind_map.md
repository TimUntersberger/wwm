# bind_map

Defines a new keybinding for each key in the map, where the key has the provided modifier
prepended and the keybinding calls the provided callback with its value.

```
enum KeybindingKind {
    /// always active
    Global = "global",
    /// active when in work mode
    Work = "work",
    /// default
    Normal = "normal"
}
```
## Signature

```nogscript
fn bind_map(modifier: String, callback: (), map: Map<String,, always_active: KeybindingKind?) -> Void
```

## Example

```nogscript
 nog.bind_map("Alt", nog.workspace.focus, #{
   "H": "Left",
   "J": "Down",
   "K": "Up",
   "L": "Right"
 }, "work")
```

