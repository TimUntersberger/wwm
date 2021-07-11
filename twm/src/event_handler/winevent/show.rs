use crate::{system::NativeWindow, system::SystemResult, AppState};
use crate::config::rule::Action as RuleAction;
use log::{debug, error};

pub fn handle(state: &mut AppState, mut window: NativeWindow, force: bool) -> SystemResult {
    let min_width = state.config.min_width;
    let min_height = state.config.min_height;

    let config = state.config.clone();
    let rect = fail!(window
        .get_rect()
        .map_err(|_| "Failed to get rectangle of new window"));

    let too_small = rect.right - rect.left < min_width || rect.bottom - rect.top < min_height;

    let grid_allows_managing = {
        let display = state.get_current_display();
        if let Some(grid) = display.get_focused_grid() {
            !config.ignore_fullscreen_actions || !grid.is_fullscreened()
        } else {
            false
        }
    };

    let rules = config
        .rules
        .iter()
        .chain(state.additonal_rules.iter())
        .collect();

    window.set_matching_rule(rules);

    let parent = window.get_parent_window();
    let rule = window.rule.clone().unwrap_or_default();
    let is_pinned_window = state.pinned_windows.contains_key(&window.id.into());
    let should_manage = force || rule.action == RuleAction::Manage || (rule.action == RuleAction::Validate && !too_small && parent.is_err() && window.should_manage() && grid_allows_managing);

    if rule.action == RuleAction::Pin {
        state.pin_window(window.id.into())?;
        state.store_pinned();
    } else if should_manage && !is_pinned_window {
        debug!("Managing window");
        if rule.workspace_id != -1 {
            state.change_workspace(rule.workspace_id, false)?;
        }

        window.init(config.remove_title_bar, config.use_border)?;

        let display = state.get_current_display_mut();
        if let Some(grid) = display.get_focused_grid_mut() {
            grid.push(window);
        }
        display.refresh_grid(&config)?;
    }

    Ok(())
}
