use crate::window::Window;
use crate::WORKSPACE_ID;
use crate::GRIDS;
use crate::CONFIG;
use crate::util;
use crate::change_workspace;
use crate::window::gwl_style::GwlStyle;
use crate::window::gwl_ex_style::GwlExStyle;

use winapi::shared::windef::HWND;
use log::debug;

pub fn handle(hwnd: HWND, ignore_window_style: bool) -> Result<(), Box<dyn std::error::Error>> {
    // gets the GWL_STYLE of the window. GWL_STYLE returns a bitmask that can be used to find out attributes about a window
    //TODO: fix problem with powershell and native wsl
    let mut window = Window {
        id: hwnd as i32,
        title: util::get_title_of_window(hwnd)?,
        ..Window::default()
    };
    window.original_style = window.get_style().unwrap_or(GwlStyle::default());

    let exstyle = window.get_ex_style().unwrap_or(GwlExStyle::default());
    let parent = window.get_parent_window();


    let correct_style = ignore_window_style
        || (window.original_style.contains(GwlStyle::CAPTION)
            && !exstyle.contains(GwlExStyle::DLGMODALFRAME));

    for rule in &CONFIG.rules {
        if rule.pattern.is_match(&window.title) {
            debug!("Rule({:?}) matched!", rule.pattern);
            window.rule = Some(rule.clone());
            break;
        }
    }

    let should_manage = window.rule.clone().unwrap_or_default().manage && parent.is_err() && correct_style;

    if should_manage {
        debug!("Managing window");
        let rule = window.rule.clone().unwrap_or_default();
        let mut workspace_id = *WORKSPACE_ID.lock().unwrap();

        if rule.workspace != -1 {
            workspace_id = rule.workspace;
            change_workspace(workspace_id)?;
        }

        if CONFIG.remove_title_bar {
            window.remove_title_bar()?;
        }

        let mut grids = GRIDS.lock().unwrap();
        let grid = grids
            .iter_mut()
            .find(|g| g.id == workspace_id)
            .unwrap();

        window.original_rect = window.get_rect()?;

        grid.split(window);

        grid.draw_grid();
    }

    Ok(()) 
}