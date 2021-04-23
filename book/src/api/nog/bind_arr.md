# bind_arr

Defines a new keybinding for each item in the array, where the key is the modifier + index and
the keybinding calls the provided callback with the item.

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
fn bind_arr(modifier: String, callback: (), arr: Any[], always_active: KeybindingKind?) -> Void
```

## Example

```nogscript
 nog.bind_arr("Alt", nog.workspace.change, range(10), "work")
```

