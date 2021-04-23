# bind

Defines a new keybinding that calls the callback when the given key combo is pressed.

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
fn bind(key_combo: String, callback: (), kind: KeybindingKind?) -> Void
```

## Example

```nogscript
 nog.bind("F1", () => print("Hello World"), "global")
```

