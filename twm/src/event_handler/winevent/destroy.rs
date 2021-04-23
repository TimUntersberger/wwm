use crate::{system::NativeWindow, system::SystemResult, AppState};

pub fn handle(
    state: &mut AppState,
    window: NativeWindow,
    _grid_id: Option<i32>, // TODO: maybe remove this? IDK
) -> SystemResult {
    if let Some(_) = state
        .find_grid_containing_window(window.id)
        .map(|g| g.remove_by_window_id(window.id))
    {
        state.refresh_visible_grids()?;
    }
    Ok(())
}
