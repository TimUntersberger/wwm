use crate::{
    config::Config,
    direction::Direction,
    display::Display,
    renderer::{NativeRenderer, Renderer},
    split_direction::SplitDirection,
    system::NativeWindow,
    system::SystemError,
    system::SystemResult,
    system::WindowId,
    tile_grid::{
        graph_wrapper::GraphWrapper, node::Node, node::NodeInfo, text_renderer::TextRenderer,
        tile_render_info::TileRenderInfo,
    },
};
use log::{debug, error, info};
use std::cmp;
use std::collections::HashMap;

pub mod graph_wrapper;
pub mod node;
pub mod store;
pub mod text_renderer;
pub mod tile_render_info;

static FULL_SIZE: u32 = 120;
static HALF_SIZE: u32 = FULL_SIZE / 2;

#[derive(Clone, Debug)]
pub struct TileGrid<TRenderer: Renderer = NativeRenderer> {
    pub renderer: TRenderer,
    pub id: i32,
    pub taskbar_window: i32,
    pub focused_id: Option<usize>,
    pub fullscreen_id: Option<usize>,
    pub next_axis: SplitDirection,
    // TODO: currently defaults to Right/Down but implementing a toggle on this would allow
    //       pushing a tile "above" a focused tile in a column or "before" a focused tile in a row
    //       as opposed to the current way where it always adds below/after
    pub next_direction: Direction,
    graph: GraphWrapper,
    pub pinned_windows: HashMap<i32, NativeWindow>,
}

impl TileGrid {
    pub fn draw_grid(&self, display: &Display, config: &Config) -> SystemResult {
        // for Debug purposes. Adds information to the log
        // TODO: make this configurable?
        debug!(
            "IsFullScreened? {} FocusedNode: {:?}",
            self.fullscreen_id.is_some(),
            self.focused_id
        );
        let render_infos = self.get_render_info(64, 20);
        debug!("{}", TextRenderer::render(64, 20, render_infos));

        let (padding, margin) = (
            if config.inner_gap > 0 {
                config.inner_gap / 2
            } else {
                0
            },
            if config.outer_gap > 0 {
                config.outer_gap
            } else {
                0
            },
        );

        let display_width = display.working_area_width(config) - margin;
        let display_height = display.working_area_height(config) - margin;
        let display_left = display.working_area_left() + (margin / 2);
        let display_top = display.working_area_top(config) + (margin / 2);

        let render_infos = self.get_render_info(display_width as u32, display_height as u32);

        info!("Beginning Rendering");
        for render_info in render_infos {
            let left_padding = if render_info.x != 0 { padding } else { 0 };
            let top_padding = if render_info.y != 0 { padding } else { 0 };
            let right_padding = if (render_info.x + render_info.width) as i32 != display_width {
                padding
            } else {
                0
            };
            let bottom_padding = if (render_info.y + render_info.height) as i32 != display_height {
                padding
            } else {
                0
            };

            let left = display_left + render_info.x as i32 + left_padding;
            let top = display_top + render_info.y as i32 + top_padding;
            let width = render_info.width as i32 - left_padding - right_padding;
            let height = render_info.height as i32 - top_padding - bottom_padding;

            self.renderer.render(
                self,
                &render_info.window,
                config,
                display,
                left,
                top,
                width,
                height,
            )?;
        }
        info!("Rendering completed");

        Ok(())
    }
    /// Returns a list of render information for each tile in the graph
    /// inner/outer padding should be handled outside of the tile grid by reducing the
    /// width/height by the outer padding and trimming off between tiles with the inner padding.
    pub fn get_render_info(&self, width: u32, height: u32) -> Vec<TileRenderInfo> {
        let mut render_infos = Vec::<TileRenderInfo>::new();

        if let Some(fullscreen_id) = self.fullscreen_id {
            match self.graph.node(fullscreen_id) {
                Node::Tile((node, window)) => {
                    render_infos.push(TileRenderInfo {
                        window: window.clone(),
                        x: 0,
                        y: 0,
                        height: height,
                        width: width,
                        debug_id: fullscreen_id,
                        debug_size: node.size,
                        debug_order: node.order,
                    });
                }
                _ => (),
            }
        } else if let Some(root_id) = self.graph.get_root() {
            render_infos = self.populate_render_info(render_infos, root_id, 0, width, 0, height);
        }

        render_infos
    }
    /// A recursive function that walks the graph and populates the supplied vec with rendering information
    /// for each node based on the given resolution.
    fn populate_render_info(
        &self,
        mut render_infos: Vec<TileRenderInfo>,
        current_node_id: usize,
        min_x: u32,
        max_x: u32,
        min_y: u32,
        max_y: u32,
    ) -> Vec<TileRenderInfo> {
        match self.graph.node(current_node_id) {
            Node::Tile((node, window)) => {
                render_infos.push(TileRenderInfo {
                    window: window.clone(),
                    x: min_x,
                    y: min_y,
                    height: if min_y > max_y { 0 } else { max_y - min_y },
                    width: if min_x > max_x { 0 } else { max_x - min_x },
                    debug_id: current_node_id,
                    debug_size: node.size,
                    debug_order: node.order,
                });
            }
            Node::Column(_) => {
                let children = self.graph.get_sorted_children(current_node_id);
                let length = children.len();
                let mut current_min_x = min_x;
                let mut remainder = (max_x - min_x) % children.len() as u32;
                let mut get_remainder_slice = || {
                    if remainder > 0 {
                        remainder -= 1;
                        1
                    } else {
                        0
                    }
                };

                let mut count = 1;
                for child in children {
                    let child_size = self.graph.node(child).get_size();
                    let item_width = (((max_x - min_x) as f32)
                        * (child_size as f32 / FULL_SIZE as f32))
                        .floor() as u32;

                    if item_width <= max_x {
                        let remainder_slice = get_remainder_slice();
                        let current_max_x = if count == length {
                            max_x
                        } else {
                            current_min_x + item_width + remainder_slice
                        };

                        render_infos = self.populate_render_info(
                            render_infos,
                            child,
                            current_min_x,
                            current_max_x,
                            min_y,
                            max_y,
                        );
                        current_min_x += item_width + remainder_slice;
                    }

                    count += 1;
                }
            }
            Node::Row(_) => {
                let children = self.graph.get_sorted_children(current_node_id);
                let length = children.len();
                let mut current_min_y = min_y;
                let mut remainder = (max_y - min_y) % children.len() as u32;
                let mut get_remainder_slice = || {
                    if remainder > 0 {
                        remainder -= 1;
                        1
                    } else {
                        0
                    }
                };

                let mut count = 1;
                for child in children {
                    let child_size = self.graph.node(child).get_size();
                    let item_height = (((max_y - min_y) as f32)
                        * (child_size as f32 / FULL_SIZE as f32))
                        .floor() as u32;

                    if item_height <= max_y {
                        let remainder_slice = get_remainder_slice();
                        let current_max_y = if count == length {
                            max_y
                        } else {
                            current_min_y + item_height + remainder_slice
                        };

                        render_infos = self.populate_render_info(
                            render_infos,
                            child,
                            min_x,
                            max_x,
                            current_min_y,
                            current_max_y,
                        );
                        current_min_y += item_height + remainder_slice;
                    }

                    count += 1;
                }
            }
        }

        render_infos
    }
}

impl<TRenderer: Renderer> TileGrid<TRenderer> {
    pub fn new(id: i32, renderer: TRenderer) -> TileGrid<TRenderer> {
        Self {
            id,
            // display: get_primary_display(),
            renderer,
            taskbar_window: 0,
            graph: GraphWrapper::new(),
            fullscreen_id: None,
            focused_id: None,
            next_axis: SplitDirection::Vertical,
            next_direction: Direction::Right,
            pinned_windows: HashMap::new(),
        }
    }
    /// Returns whether the tile grid is populated or not
    pub fn is_empty(&self) -> bool {
        self.graph.is_empty()
    }
    /// Returns whether the tile grid is fullscreened or not
    pub fn is_fullscreened(&self) -> bool {
        self.fullscreen_id.is_some()
    }
    /// Iterates and hides every window managed by the current tile grid
    pub fn hide(&self) {
        for node_id in self.graph.nodes() {
            if self.graph.node(node_id).is_tile() {
                self.graph.node(node_id).get_window().hide();
            }
        }
    }
    /// Removes the focused node, if it exists, and returns the window on that node.
    /// Leaves the tile_grid in an unfocused state and un-fullscreens if currently fullscreened.
    pub fn pop(&mut self) -> Option<NativeWindow> {
        let removed_node: Option<Node> = self.remove_node(self.focused_id);
        self.focused_id = None;
        self.fullscreen_id = None;

        removed_node.map(|x| x.take_window())
    }
    /// Calls cleanup on all managed windows and clears the tile_grid
    pub fn cleanup(&mut self) -> SystemResult {
        self.modify_windows(|window| window.cleanup())?;
        self.graph.clear();
        self.focused_id = None;
        self.fullscreen_id = None;

        Ok(())
    }
    /// Sets the currently focused tile to be fullscreened if it's not already,
    /// otherwise reverts the graph to non-fullscreened mode.
    pub fn toggle_fullscreen(&mut self) {
        if self.focused_id.is_some() {
            if self.fullscreen_id.is_some() {
                self.fullscreen_id = None;
            } else {
                self.fullscreen_id = self.focused_id;
            }
        }
    }
    /// Travels up the graph from the focused node until it finds a row
    /// and then resets the size of all of that row's children.
    /// No-op if no row is found above the focused node.
    pub fn reset_row(&mut self) {
        self.reset_size(self.graph.to_closest_row(self.focused_id));
    }
    /// Travels up the graph from the focused node until it finds a column
    /// and then resets the size of all of that column's children.
    /// No-op if no column is found above the focused node.
    pub fn reset_column(&mut self) {
        self.reset_size(self.graph.to_closest_column(self.focused_id));
    }
    /// Gets all the child nodes of a node and re-distrbutes the size among them.
    /// This applies only one level down, regardless of what type of nodes they are; any
    /// child Row/Column nodes' children will retain their respective size.
    fn reset_size(&mut self, parent_id: Option<usize>) {
        if !parent_id.is_some() {
            return;
        }

        let children = self.graph.get_sorted_children(parent_id.unwrap());
        let number_of_children = children.len();
        let size_per_child = FULL_SIZE / number_of_children as u32;

        let mut remainder = FULL_SIZE % number_of_children as u32;
        let mut get_remainder_slice = || {
            if remainder > 0 {
                remainder -= 1;
                1
            } else {
                0
            }
        };

        for child in children {
            self.graph
                .node_mut(child)
                .set_size(size_per_child + get_remainder_slice());
        }
    }
    /// Iterates and shows every window managed by the current tile_grid
    pub fn show(&self) -> SystemResult {
        let mut nodes = self.graph.nodes().collect::<Vec<usize>>();
        if self.fullscreen_id.is_some() {
            nodes.sort_by_key(|n| {
                if self.fullscreen_id.unwrap() == *n {
                    1
                } else {
                    0
                }
            });
        }
        for node_id in nodes {
            if self.graph.node(node_id).is_tile() {
                let window = self.graph.node(node_id).get_window();
                window.show();
                window
                    .to_foreground(true)
                    .map_err(SystemError::ShowWindow)?;
                if let Err(e) = window.remove_topmost() {
                    error!("{}", e);
                }
            }
        }

        if let Some(focused_id) = self.focused_id {
            match self.graph.node(focused_id).get_window().focus() {
                Err(_) => info!("Failed focusing window in node {}", focused_id),
                _ => (),
            }
        }
        Ok(())
    }
    /// Returns the window of the currently focused tile if it exists
    pub fn get_focused_window(&self) -> Option<&NativeWindow> {
        self.focused_id.map(|id| self.graph.node(id).get_window())
    }
    /// Returns the window that matches by ID if it exists
    pub fn get_window(&self, id: WindowId) -> Option<&NativeWindow> {
        self.graph
            .nodes()
            .find(|n| {
                let node = self.graph.node(*n);
                node.is_tile() && node.get_window().id == id
            })
            .map(|n| self.graph.node(n).get_window())
    }
    /// Runs the passed in function on the currently focused tile's window in the current tile grid.
    pub fn modify_focused_window<TFunction>(self: &mut Self, f: TFunction) -> SystemResult
    where
        TFunction: FnMut(&mut NativeWindow) -> SystemResult + Copy,
    {
        if let Some(focused_id) = self.focused_id {
            self.graph.node_mut(focused_id).modify_window(f)?;
        }
        Ok(())
    }
    /// Calls the passed in function for each tile node with a mutable ref to the node. Useful for
    /// changing all windows in the current tile grid.
    pub fn modify_windows<TFunction>(self: &mut Self, f: TFunction) -> SystemResult
    where
        TFunction: FnMut(&mut NativeWindow) -> SystemResult + Copy,
    {
        for node_id in self.graph.nodes() {
            let node = self.graph.node_mut(node_id);
            if node.is_tile() {
                node.modify_window(f)?;
            }
        }
        Ok(())
    }
    /// Returns a list of immutable window references.
    pub fn get_windows(self: &Self) -> Vec<&NativeWindow> {
        self.graph
            .nodes()
            .map(|id| self.graph.node(id))
            .filter(|node| node.is_tile())
            .map(|node| node.get_window())
            .collect()
    }
    /// Swaps position of the focused tile with the tile in the supplied direction. See swap for more details on behavior.
    pub fn swap_focused(&mut self, direction: Direction) {
        if let Some(focused_id) = self.focused_id {
            self.swap(focused_id, direction);
        }
    }
    /// Swaps position of the given tile with the tile in the supplied direction. No-op if grid has no focused tile.
    /// If the direction doesn't make sense, like swapping to the up/down in a column tile OR swapping
    /// left/right in a row tile, then the swap is propagated up the tree to the next parent that is able to swap in
    /// the given direction.
    pub fn swap(&mut self, node_id: usize, direction: Direction) {
        if let Some(parent_id) = self.graph.map_to_parent(Some(node_id)) {
            let selected_node_order = self.graph.node(node_id).get_order();
            let children = self.graph.get_children(parent_id);

            let should_swap_with_sibling = match (&direction, self.graph.node(parent_id)) {
                (Direction::Left, Node::Column(_)) | (Direction::Up, Node::Row(_)) => {
                    selected_node_order > 0 && children.len() > 1
                }
                (Direction::Right, Node::Column(_)) | (Direction::Down, Node::Row(_)) => {
                    selected_node_order < (children.len() - 1) as u32
                }
                _ => false,
            };

            if should_swap_with_sibling {
                let sibling_id = self.graph.get_neighbor(node_id, direction);
                self.swap_order(node_id, sibling_id.unwrap());
            } else {
                // bubble up the swap to the parent
                self.swap(parent_id, direction);
            }
        }
    }
    /// Trades order (the index/position in a column or row) between two nodes. This function assumes the given nodes are siblings.
    fn swap_order(&mut self, first: usize, second: usize) {
        let first_order = self.graph.node(first).get_order();
        let second_order = self.graph.node(second).get_order();

        self.graph.node_mut(first).set_order(second_order);
        self.graph.node_mut(second).set_order(first_order);
    }
    /// Used to switch focus from the focused tile to the next tile in the given direction.
    /// no-op if there is no tile focused. If a sibling is found in
    /// the given direction then focus is moved to the sibling. Otherwise, the function travels
    /// up the graph checking each parents' children to see if there is an applicable sibling to switch
    /// focus to until it hits the root node at which point it exits leaving focus unchanged.
    /// This allows focus to be switched up/down rows but also doing a focus left/right moves to the
    /// next-closest column in the given direction and vice versa for columns.
    pub fn focus(&mut self, direction: Direction) -> SystemResult {
        if !self.focused_id.is_some() {
            return Ok(());
        }

        let parent_id = self.graph.map_to_parent(self.focused_id);
        if let Some(mut parent_id) = parent_id {
            let mut target_focus: Option<usize> = None;
            let mut current_focus = self.focused_id.unwrap();
            while !target_focus.is_some() {
                let children = self.graph.get_children(parent_id).len();
                let focused_order = self.graph.node(current_focus).get_order();

                let should_focus_sibling = match (&direction, self.graph.node(parent_id)) {
                    (Direction::Left, Node::Column(_)) | (Direction::Up, Node::Row(_)) => {
                        focused_order > 0 && children > 1
                    }
                    (Direction::Right, Node::Column(_)) | (Direction::Down, Node::Row(_)) => {
                        focused_order < (children - 1) as u32
                    }
                    _ => false,
                };

                if should_focus_sibling {
                    target_focus = self.graph.get_neighbor(current_focus, direction);
                } else if let Some(p_id) = self.graph.map_to_parent(Some(parent_id)) {
                    // focus on parent and iterate again to find a tile in chosen direction
                    current_focus = parent_id;
                    parent_id = p_id;
                } else {
                    // no parent, can't move in direction
                    target_focus = self.focused_id;
                }
            }

            self.focused_id = self.graph.to_closest_tile(target_focus, Some(direction));
            self.graph
                .node(self.focused_id.unwrap())
                .get_window()
                .focus()?;
        }

        if self.is_fullscreened() {
            self.fullscreen_id = self.focused_id;
        }

        Ok(())
    }
    /// Resets the order of all child nodes by sorting them and then "re-indexing" their order starting at 0
    fn reset_order(&mut self, parent_id: usize) {
        let nodes = self.graph.get_sorted_children(parent_id);

        let mut order = 0;
        for node in nodes {
            self.graph.node_mut(node).set_order(order);
            order += 1;
        }
    }
    /// Removes and returns the node of the given node_id. The behavior of the removal falls into one of three cases:
    /// Case One: If the graph only has one node and it's the given node, then the graph is emptied.
    /// Case Two: If the given node has only one sibling then the node is removed and its sibling gets propogated
    /// "up" a level to take the place of its parent node (A parent node with only one child is an invalid state).
    /// If the sibling is a Column or Row and its parent matches its type, then the Column/Row sibling node
    /// also gets removed and its children get added to the parent node (the grandparent of the focused node)
    /// Case Three: If the given node has more than one sibling then the node is removed and its size is distributed among its siblings
    fn remove_node(&mut self, node_id: Option<usize>) -> Option<Node> {
        let mut removed_node: Option<Node> = None;
        if let Some(current_id) = node_id {
            if let Some(parent_id) = self.graph.map_to_parent(Some(current_id)) {
                let children = self.graph.get_children(parent_id);
                let number_of_children = children.len();
                if number_of_children == 2 {
                    // remove the current item
                    // make the other child take place of parent
                    let sibling_id = *children.iter().find(|x| **x != current_id).unwrap();
                    let (order, size) = self.graph.node(parent_id).get_info();

                    if let Some(grand_parent_id) = self.graph.map_to_parent(Some(parent_id)) {
                        match (
                            self.graph.node(grand_parent_id),
                            self.graph.node(sibling_id),
                        ) {
                            (Node::Column(_), Node::Column(_)) | (Node::Row(_), Node::Row(_)) => {
                                // here we need to distribute the size of the parent
                                // across the children of the sibling based on their
                                // existing ratios and then add them to the grand_parent

                                /*
                                    Removing * node (shows size distributions)
                                    c120                c120
                                    /  \                / | \
                                  t60   r60        ->  60 20 20
                                   1   /   \            1  3  4
                                     *t60  c60
                                       2   / \
                                         t60 t60
                                          3   4
                                */

                                let mut size_remaining = size;
                                let children = self.graph.get_sorted_children(sibling_id);
                                let number_of_children = children.len();
                                for (i, child_id) in children.iter().rev().enumerate() {
                                    self.graph.disconnect(sibling_id, *child_id);
                                    let child_node = self.graph.node_mut(*child_id);
                                    let mut new_size = ((child_node.get_size() as f32
                                        / FULL_SIZE as f32)
                                        * size as f32)
                                        as u32;

                                    size_remaining -= new_size;
                                    if number_of_children - 1 == i {
                                        new_size += size_remaining;
                                    }

                                    child_node.set_info(order, new_size);
                                    self.graph.connect(grand_parent_id, *child_id);
                                }

                                self.reset_order(grand_parent_id);
                                self.graph.remove_node(sibling_id);
                            }
                            _ => {
                                self.graph.connect(grand_parent_id, sibling_id);
                                let keep_node = self.graph.node_mut(sibling_id);
                                keep_node.set_info(order, size);
                            }
                        }
                    }

                    self.graph.remove_node(parent_id);
                    removed_node = self.graph.remove_node(current_id);
                } else {
                    // remove the current item
                    // distribute size among siblings
                    self.distribute_size_among_siblings(parent_id, current_id);
                    removed_node = self.graph.remove_node(current_id);
                    self.reset_order(parent_id);
                }
            } else {
                // focused is root node so empy out entire graph
                removed_node = self.graph.remove_node(current_id);
                self.graph.clear();
            }
        }

        removed_node
    }
    pub fn close_focused(&mut self) -> Option<NativeWindow> {
        if let Some(focused_node) = self.focused_id.map(|id| self.graph.node(id)) {
            let window_id = focused_node.get_window().id;
            self.remove_by_window_id(window_id);
        }

        None
    }
    pub fn remove_by_window_id(&mut self, id: WindowId) -> Option<NativeWindow> {
        let mut window: Option<NativeWindow> = None;
        if let Some(node_id) = self.graph.find(|x| x.is_tile() && x.get_window().id == id) {
            window = self.remove_node(Some(node_id)).map(|x| x.take_window());
            if let Some(focused_id) = self.focused_id {
                if focused_id == node_id {
                    self.focused_id = None;
                }
            }
            if let Some(fullscreen_id) = self.fullscreen_id {
                if fullscreen_id == node_id || self.graph.nodes().count() <= 1 {
                    self.fullscreen_id = None;
                }
            }
        }

        window
    }
    /// Returns whether a given window ID exists in the tile grid
    pub fn contains(&self, window_id: WindowId) -> bool {
        self.graph
            .nodes()
            .find(|n| {
                let node = self.graph.node(*n);
                node.is_tile() && node.get_window().id == window_id
            })
            .is_some()
    }
    /// Sets the currently focused tile to whatever happens to be "last" in the graph.
    /// See get_last_tile for more information.
    pub fn focus_last_tile(self: &mut Self) {
        self.focused_id = self.get_last_tile();
    }
    /// Returns the an Option NodeID (usize) of the last Tile in the tile grid.
    /// This is somewhat arbitrary as it won't necessarily be the last node added to
    /// the grid based on the graph implementation but can serve as a "give me a node toward the 'bottom'
    /// of the graph."
    fn get_last_tile(&self) -> Option<usize> {
        self.graph
            .nodes()
            .filter(|n| self.graph.node(*n).is_tile())
            .last()
    }
    /// Focuses the tile that holds the given window ID if it exists in the current tile grid
    pub fn focus_tile_by_window_id(&mut self, window_id: WindowId) {
        let maybe_window_tile = self.graph.nodes().find(|n| {
            let node = self.graph.node(*n);
            node.is_tile() && node.get_window().id == window_id
        });
        if maybe_window_tile.is_some() {
            self.focused_id = maybe_window_tile;
        }
    }
    /// Creates a node from the given window and adds it to the graph if the grid doesn't already contain the window.
    /// If the grid doesn't have a focused window, it resorts to focusing the last tile in the grid.
    /// Pushing a tile depends on the state of the focused tile. If the focused tile is part of a column or row "list"
    /// then it gets appended to the list next to the focused tile (other siblings get their order updated). If the focused
    /// tile doesn't have a sibling then the function introduces a new parent node opposite of the current parent's type
    /// and nests the focused node and the new window node within. This is how pushing into a tile creates rows or columns.
    pub fn push(&mut self, window: NativeWindow) {
        if self.graph.len() == 0 {
            let new_root_node = Node::Tile((
                NodeInfo {
                    order: 0,
                    size: FULL_SIZE,
                },
                window,
            ));
            self.focused_id = Some(self.graph.add_node(new_root_node));

            // first node inserted in empty graph so return early
            return;
        }

        if self.contains(window.id) {
            // window is already in graph
            return;
        }

        if !self.focused_id.is_some() {
            // if we're not focused, just focus last tile in the graph
            self.focused_id = self.get_last_tile();
        }

        if let Some(current_id) = self.focused_id {
            let mut new_node = Node::Tile((NodeInfo { order: 0, size: 0 }, window));
            // determines whether to add the tile before or after the currently focused tile
            let (existing_node_order, new_node_order) = match self.next_direction {
                Direction::Up | Direction::Left => (1, 0),
                _ => (0, 1),
            };
            match self.graph.node(current_id) {
                Node::Tile(_) => {
                    if let Some(parent_id) = self.graph.map_to_parent(Some(current_id)) {
                        // locally-scoped fn type and enum used to keep track of how the new node gets added
                        // (whether it gets added to a column/row and whether it should nest with an existing
                        // tile or get inserted into an existing column/row)
                        type CreateNode = fn(u32, u32) -> Node;
                        enum PushOperation {
                            AppendToParent,
                            SwapAndAppend(CreateNode),
                        }

                        // this determines how the node should be added
                        let operation = match (self.graph.node(parent_id), self.next_axis) {
                            (Node::Column(_), SplitDirection::Vertical)
                            | (Node::Row(_), SplitDirection::Horizontal) => {
                                PushOperation::AppendToParent
                            }
                            (Node::Column(_), _) => PushOperation::SwapAndAppend(Node::row),
                            (Node::Row(_), _) => PushOperation::SwapAndAppend(Node::column),
                            _ => {
                                error!("Parent not column or row");
                                return;
                            }
                        };

                        match operation {
                            PushOperation::AppendToParent => {
                                // parent is same type as what we want to add
                                // so append item to parent's list
                                let (current_node_order, ..) =
                                    self.graph.node(current_id).get_info();
                                let new_node_order = current_node_order + new_node_order;
                                new_node
                                    .set_info(new_node_order, self.make_space_for_node(parent_id));
                                self.shift_order(parent_id, new_node_order);
                                self.focused_id = Some(self.graph.add_child(parent_id, new_node));
                            }
                            PushOperation::SwapAndAppend(create_node) => {
                                // parent is opposite type of what we want to add
                                // so swap current node with opposite of the parent node's type
                                // and append current item + new item there. This is how we can
                                // nest columns in rows or rows in columns.
                                let (new_order, new_size) = self.graph.node(current_id).get_info();
                                let new_parent_node = create_node(new_order, new_size);

                                let (new_parent_id, child_id) =
                                    self.graph.swap_and_nest(current_id, new_parent_node);
                                self.graph
                                    .node_mut(child_id)
                                    .set_info(existing_node_order, HALF_SIZE);
                                new_node.set_info(new_node_order, HALF_SIZE);
                                self.focused_id =
                                    Some(self.graph.add_child(new_parent_id, new_node));
                            }
                        }
                    } else
                    /* must be root tile */
                    {
                        let new_parent = match self.next_axis {
                            SplitDirection::Vertical => Node::column(0, FULL_SIZE),
                            SplitDirection::Horizontal => Node::row(0, FULL_SIZE),
                        };

                        let (new_parent_id, child_id) =
                            self.graph.swap_and_nest(current_id, new_parent);
                        self.graph
                            .node_mut(child_id)
                            .set_info(existing_node_order, HALF_SIZE);
                        new_node.set_info(new_node_order, HALF_SIZE);
                        self.focused_id = Some(self.graph.add_child(new_parent_id, new_node));
                    }
                }
                _ => error!("Focused node not a tile. This is an invalid state"),
            }
        }
    }
    /// Increments the "order" index of all siblings starting from the given shift_point.
    /// Used for moving all sibling nodes after a point to the right/down to make room for a new node.
    fn shift_order(&mut self, parent_id: usize, mut shift_point: u32) {
        let nodes = self.graph.get_sorted_children(parent_id);
        let nodes = nodes
            .iter()
            .filter(|x| self.graph.node(**x).get_info().0 >= shift_point)
            .collect::<Vec<_>>();

        for node in nodes {
            shift_point += 1;
            self.graph.node_mut(*node).set_order(shift_point);
        }
    }
    fn make_space_for_node(&mut self, parent_id: usize) -> u32 {
        let mut children = self.graph.get_children(parent_id);
        let target_size_of_new_item = (FULL_SIZE as f32 / (children.len() as f32 + 1.0)).floor();
        let mut existing_children_total = 0;

        let take_from_each = (target_size_of_new_item / children.len() as f32) as u32;
        let mut remainder = (target_size_of_new_item % children.len() as f32) as u32;
        let mut take_remainder = || {
            if remainder > 0 {
                remainder -= 1;
                1
            } else {
                0
            }
        };

        children.sort_by_key(|x| self.graph.node(*x).get_size());
        children.reverse();

        for child_id in children {
            let mut child_size = self.graph.node(child_id).get_size();
            child_size -= take_from_each + take_remainder();

            existing_children_total += child_size;
            self.graph.node_mut(child_id).set_size(child_size);
        }

        FULL_SIZE - existing_children_total
    }
    pub fn trade_size_with_neighbor(
        &mut self,
        node_id: Option<usize>,
        direction: Direction,
        size: i32,
    ) {
        if !node_id.is_some() {
            return;
        }

        if let Some(parent_id) = self.graph.map_to_parent(node_id) {
            let node_id = node_id.unwrap();
            let (node_order, node_size) = self.graph.node(node_id).get_info();
            let children = self.graph.get_children(parent_id);

            match (direction, &self.graph.node(parent_id)) {
                (Direction::Left, Node::Column(_)) | (Direction::Up, Node::Row(_))
                    if node_order > 0 =>
                {
                    if let Some(neighbor_id) = children
                        .iter()
                        .find(|x| self.graph.node(**x).get_order() == node_order - 1)
                    {
                        let neighbor_size = self.graph.node(*neighbor_id).get_size();

                        if size > 0 && neighbor_size > size.abs() as u32
                            || size < 0 && node_size > size.abs() as u32
                        {
                            self.graph
                                .node_mut(*neighbor_id)
                                .set_size((neighbor_size as i32 - size) as u32);
                            self.graph
                                .node_mut(node_id)
                                .set_size((node_size as i32 + size) as u32);
                        }
                    }
                }
                (Direction::Right, Node::Column(_)) | (Direction::Down, Node::Row(_)) => {
                    if let Some(neighbor_id) = children
                        .iter()
                        .find(|x| self.graph.node(**x).get_order() == node_order + 1)
                    {
                        let neighbor_size = self.graph.node(*neighbor_id).get_size();

                        if size > 0 && neighbor_size > size.abs() as u32
                            || size < 0 && node_size > size.abs() as u32
                        {
                            self.graph
                                .node_mut(*neighbor_id)
                                .set_size((neighbor_size as i32 - size) as u32);
                            self.graph
                                .node_mut(node_id)
                                .set_size((node_size as i32 + size) as u32);
                        }
                    }
                }
                _ => self.trade_size_with_neighbor(Some(parent_id), direction, size),
            }
        }
    }
    /// Moves the focused tile out of a row/column in the given direction and automatically handles redistributing size/resetting the order
    /// of any previous siblings. The behavior of this movement is essentially moving the tile so that it is a sibling of its parent and introducing a
    /// new parent node that is the opposite type of the previous parent if necessary.
    pub fn move_focused_out(&mut self, direction: Direction) {
        if let Some(parent_id) = self.graph.map_to_parent(self.focused_id) {
            let focused_id = self.focused_id.unwrap();
            let children = self.graph.get_sorted_children(parent_id);

            // This block handles when the focused tile is directly under the root node
            if !self.graph.map_to_parent(Some(parent_id)).is_some() {
                let new_root = match self.graph.node(parent_id) {
                    Node::Column(_) => Node::row(0, FULL_SIZE),
                    Node::Row(_) => Node::column(0, FULL_SIZE),
                    _ => {
                        error!("Parent must be row or column");
                        return;
                    }
                };
                if children.len() == 2
                    && self.graph.node(children[0]).is_tile()
                    && self.graph.node(children[1]).is_tile()
                {
                    // This is a weird case since moving out doesn't make sense when you only have two tile nodes in the grid
                    // Rather than doing nothing, this swaps the root with a new root that is the opposite of the current root type.
                    // And swaps the order of the two children if the user is attempting to move a tile to the other side of its sibling
                    self.graph.swap_node(parent_id, new_root);
                    let (focused_child, other_child) = if focused_id == children[0] {
                        (children[0], children[1])
                    } else {
                        (children[1], children[0])
                    };
                    let focused_order = self.graph.node(focused_child).get_order();
                    let other_order = self.graph.node(other_child).get_order();
                    if (focused_order > other_order
                        && (direction == Direction::Left || direction == Direction::Up))
                        || (focused_order < other_order
                            && (direction == Direction::Right || direction == Direction::Down))
                    {
                        self.graph.node_mut(focused_child).set_order(other_order);
                        self.graph.node_mut(other_child).set_order(focused_order);
                    }
                } else if children.len() > 2 {
                    // Creates a new root (opposite of current root type) and adds the focused tile
                    // as a child and the old root as the other. In effect, this moves the focused tile
                    // out of the existing row/column when there is only one row/column in the grid.
                    self.disconnect_child(parent_id, focused_id);
                    let new_root_id = self.graph.add_node(new_root);
                    self.graph.connect(new_root_id, parent_id);
                    self.graph.connect(new_root_id, focused_id);

                    // This determines the sibling order of the focused tile and the old root
                    // based on the movement direction and the new node type (column or row)
                    let (left, right) = match direction {
                        Direction::Left | Direction::Up => (focused_id, parent_id),
                        Direction::Right | Direction::Down => (parent_id, focused_id),
                    };
                    self.graph.node_mut(left).set_info(0, HALF_SIZE);
                    self.graph.node_mut(right).set_info(1, HALF_SIZE);
                }

                return;
            }

            // The rest of the function handles the case when the focused tile is further down the graph, that is, when the focused tile has a grand parent.
            // The idea here is to determine what the new parent would be (the grand parent of the focused node) and get a reference to the current parent
            // which will be the new sibling. Then, the focused tile is appended to the new parent (previous grand parent) and the order of the
            // focused node and the sibling (old parent) is determined based on the direction given by the user.
            let (new_parent_id, sibling_id) = match (&direction, self.graph.node(parent_id)) {
                (Direction::Left, Node::Column(_))
                | (Direction::Up, Node::Row(_))
                | (Direction::Right, Node::Column(_))
                | (Direction::Down, Node::Row(_)) => (
                    self.graph
                        .map_to_parent(self.graph.map_to_parent(Some(parent_id))),
                    self.graph.map_to_parent(Some(parent_id)),
                ),
                (Direction::Up, Node::Column(_))
                | (Direction::Left, Node::Row(_))
                | (Direction::Down, Node::Column(_))
                | (Direction::Right, Node::Row(_)) => {
                    (self.graph.map_to_parent(Some(parent_id)), Some(parent_id))
                }
                _ => (None, None),
            };

            if let Some(new_parent_id) = new_parent_id {
                let sibling_id = sibling_id.unwrap();
                let sibling_order = self.graph.node(sibling_id).get_order();
                let (sibling_order, new_focused_order) = match &direction {
                    Direction::Up | Direction::Left => (sibling_order + 1, sibling_order),
                    _ => (sibling_order, sibling_order + 1),
                };

                match &self.graph.node(new_parent_id) {
                    Node::Column(_) | Node::Row(_) => {
                        self.disconnect_child(parent_id, focused_id);
                        let new_size = self.make_space_for_node(new_parent_id);
                        self.graph
                            .node_mut(focused_id)
                            .set_info(new_focused_order, new_size);
                        self.graph.node_mut(sibling_id).set_order(sibling_order);
                        self.graph.connect(new_parent_id, focused_id);
                        self.reset_order(new_parent_id);
                    }
                    _ => {
                        error!("Expected Column/Row. Tile is not a valid state");
                        return;
                    }
                }

                // this handles when a movement out of a parent creates a scenario where there is a parent (a row or column) with one child tile.
                //When this happens we just want the child to bubble up one level so that it is no longer siblingless.
                self.bubble_siblingless_child(parent_id);
            }
        }
    }
    /// Moves the focused tile into an adjacent row/column/tile found in the given direction and automatically handles redistributing size/resetting the order
    /// of any previous siblings and new siblings. If the adjacent target is a row or column, this simply appends the focused tile at the end of the container.
    /// If the adjacent target is a tile, this introduces a new column or row container, whichever is the opposite of the currently focused tile's parent, and
    /// appends the focused tile & the adjacent tile within the new container.
    pub fn move_focused_in(&mut self, direction: Direction) {
        if let Some(parent_id) = self.graph.map_to_parent(self.focused_id) {
            let focused_id = self.focused_id.unwrap();
            let number_of_children = self.graph.get_children(parent_id).len();

            if let Some(sibling_id) = self.graph.get_neighbor(focused_id, direction) {
                match &self.graph.node(sibling_id) {
                    Node::Column(_) | Node::Row(_) => {
                        // move focused under sibling column/row
                        self.disconnect_child(parent_id, focused_id);
                        let new_size = self.make_space_for_node(sibling_id);
                        let new_order = self.graph.get_children(sibling_id).len() as u32;
                        self.graph
                            .node_mut(focused_id)
                            .set_info(new_order, new_size);
                        self.graph.connect(sibling_id, focused_id);
                    }
                    Node::Tile(_) => {
                        if number_of_children == 2 {
                            // don't do anything if there are only two nodes and they're both tiles
                            // this prevents columns in columns or rows in rows
                            // in this scenario, the user should move_focused_out not move_focused_in
                            return;
                        }
                        // need to look at what the grandparent is to determine if this
                        // should make a column or row and then nest the sibling tile
                        // and append the current item to it
                        if let Some(sibling_parent_id) = self.graph.map_to_parent(Some(sibling_id))
                        {
                            let focused_order = self.graph.node(focused_id).get_order();
                            let (sibling_order, sibling_size) =
                                self.graph.node(sibling_id).get_info();
                            let new_order = cmp::min(focused_order, sibling_order);
                            let new_node = match &self.graph.node(sibling_parent_id) {
                                Node::Column(_) => Node::row(new_order, sibling_size),
                                Node::Row(_) => Node::column(new_order, sibling_size),
                                _ => {
                                    error!("Parent should be a row or column");
                                    return;
                                }
                            };

                            let new_node_id = self.graph.add_node(new_node);
                            self.graph.disconnect(sibling_parent_id, sibling_id);
                            self.graph.connect(sibling_parent_id, new_node_id);
                            self.disconnect_child(parent_id, focused_id);

                            self.graph.node_mut(sibling_id).set_info(0, HALF_SIZE);
                            self.graph.node_mut(focused_id).set_info(1, HALF_SIZE);

                            self.graph.connect(new_node_id, sibling_id);
                            self.graph.connect(new_node_id, focused_id);
                        }
                    }
                }

                self.bubble_siblingless_child(parent_id);
            }
        }
    }
    /// Scenario: moving out of a column/row leaving one child behind. This function
    /// swaps the column/row with the remaining child and deletes the column/row node
    /// Example:
    ///                  [RootColumnNode]
    ///                    /          \
    ///          [TileNodeA]          [TileNodeB]
    ///
    ///        ***Remove TileNodeA from the grid***
    ///
    ///                  [RootColumnNode]
    ///                               \
    ///                               [TileNodeB]
    ///
    ///        ***bubble_siblingless_child on RootColumnNode***
    ///
    ///
    ///                    [TileNodeB] // now the root node that has taken place of its parent
    ///
    fn bubble_siblingless_child(&mut self, parent_id: usize) {
        let children = self.graph.get_children(parent_id);
        if children.iter().len() == 1 {
            let child_id = children[0];

            if let Some(grand_parent_id) = self.graph.map_to_parent(Some(parent_id)) {
                self.graph.connect(grand_parent_id, child_id);
            }

            let (order, size) = self.graph.node(parent_id).get_info();
            self.graph.node_mut(child_id).set_info(order, size);

            self.graph.remove_node(parent_id);
        }
    }
    /// Removes conection between a parent and the given child node, but leaves the child in the graph.
    /// This is useful when moving tiles between parents.
    fn disconnect_child(&mut self, parent_id: usize, child_id: usize) {
        self.distribute_size_among_siblings(parent_id, child_id);
        self.graph.disconnect(parent_id, child_id);
        self.reset_order(parent_id);
    }
    fn distribute_size_among_siblings(&mut self, parent_id: usize, child_id: usize) {
        let children = self.graph.get_sorted_children(parent_id);
        let number_of_children = children.iter().len();
        let size = self.graph.node(child_id).get_size();
        let size_per_sibling = size / (number_of_children - 1) as u32;

        let mut remainder = size % (number_of_children - 1) as u32;
        let mut get_remainder_slice = || {
            if remainder > 0 {
                remainder -= 1;
                1
            } else {
                0
            }
        };

        for child in children {
            if child != child_id {
                let child_size = self.graph.node(child).get_size();
                self.graph
                    .node_mut(child)
                    .set_size(size_per_sibling + child_size + get_remainder_slice());
            }
        }
    }
    pub fn swap_columns_and_rows(&mut self) {
        for node_id in self.graph.nodes() {
            let node = match self.graph.node(node_id) {
                Node::Column(info) => Some(Node::row(info.order, info.size)),
                Node::Row(info) => Some(Node::column(info.order, info.size)),
                _ => None,
            };
            if let Some(n) = node {
                self.graph.swap_node(node_id, n);
            }
        }
    }
    /// Iterates nodes in tile grid and removes any that are no longer valid windows
    pub fn remove_empty_tiles(&mut self) {
        for node_id in self.graph.nodes() {
            if self.graph.node(node_id).is_tile()
                && !self.graph.node(node_id).get_window().is_window()
            {
                self.focused_id = Some(node_id);
                self.pop();
            }
        }
    }
    /// Returns a stringified version of the grid that follows this format:
    /// tiles:    t#|#|#   (t)ile (#1)order (#2)size (#3) window ID   Example: t0|60|1 (a tile with order 0, size 60 and windowID 1)
    /// columns:  c#|#[]   (c)olumn (#1)order (#2)size  [..] any children Example: c0|120[t0|60|1] (a column with order 0, size 120 and one child tile)
    /// rows:     r#|#[]   (r)olumn (#1)order (#2)size  [..] any children Example: r0|120[t0|60|1] (a row with order 0, size 120 and one child tile)
    ///     Grid          Tree                         String
    ///                     c          
    ///    11112222        / \
    ///    11113333       t1  r        c0|120[t0|60|1,r1|60[t0|40|2,t1|40|3,t2|40|4]]
    ///    11114444         / | \
    ///                   t2 t3 t4
    /// Note that the children arrays [] can nest columns and rows.
    pub fn to_string(&self) -> String {
        match self.graph.get_root() {
            Some(root) => self.inner_to_string(root),
            _ => "".into(),
        }
    }
    fn inner_to_string(&self, id: usize) -> String {
        match self.graph.node(id) {
            Node::Column(_) | Node::Row(_) => format!(
                "{}[{}]",
                self.graph.node(id).to_string(),
                self.stringify_children(id)
            ),
            Node::Tile(_) => self.graph.node(id).to_string(),
        }
    }
    fn stringify_children(&self, id: usize) -> String {
        self.graph
            .get_sorted_children(id)
            .iter()
            .map(|child_id| self.inner_to_string(*child_id))
            .collect::<Vec<String>>()
            .join(",")
    }
    /// Takes string formatted from the to_string function, parses it and populates the tile grid with the nodes and the right relationships
    /// Currently this will panic if the string isn't formatted correctly, although the strings passed into this function should be generated
    /// by the to_string function. An incorrectly formatted string would indicate a bug in the to_string function.
    pub fn from_string(&mut self, target: &String) {
        if target.len() == 0 {
            return;
        }

        self.inner_from_string(&target[..], None);

        #[cfg(not(test))] // TODO: Need to refactor Window to be able to fake calls in unit tests
        {
            self.remove_empty_tiles();
        }
    }
    fn inner_from_string(&mut self, target: &str, parent_id: Option<usize>) -> usize {
        // intended to get the matching brace when nested children occur [ [ [ ] ] ]
        //                                                               ^         ^
        let get_closing_brace_index = |s: &str| {
            let mut bracket_count: usize = 0;
            let mut index: usize = 0;

            for c in s.chars() {
                index += 1;
                match c {
                    '[' => bracket_count += 1,
                    ']' => {
                        bracket_count -= 1;
                        if bracket_count == 0 {
                            return index;
                        }
                    }
                    _ => continue,
                }
            }

            index
        };

        match target.chars().nth(0).unwrap() {
            't' => {
                // create tile node
                let end_info_index = cmp::min(
                    target.find(']').unwrap_or(target.len()),
                    target.find(',').unwrap_or(target.len()),
                );
                let tile = &target[1..end_info_index];
                let tile_information = tile.split("|").collect::<Vec<&str>>();
                let window =
                    NativeWindow::from(WindowId::from(tile_information[2].parse::<i32>().unwrap()));

                match parent_id {
                    Some(id) => {
                        let order = tile_information[0].parse::<u32>().unwrap();
                        let size = tile_information[1].parse::<u32>().unwrap();
                        let tile_node = Node::Tile((
                            NodeInfo {
                                order: order,
                                size: size,
                            },
                            window,
                        ));
                        let tile_node_id = self.graph.add_node(tile_node);
                        self.graph.connect(id, tile_node_id);
                    }
                    None => self.push(window), // simple case of just one tile in graph, so just push it in
                }

                end_info_index
            }
            character @ 'c' | character @ 'r' => {
                // create column or row node
                let end_info_index = target.find('[').unwrap();
                let node_information = &target[1..end_info_index].split("|").collect::<Vec<&str>>();
                let order = node_information[0].parse::<u32>().unwrap();
                let size = node_information[1].parse::<u32>().unwrap();
                let node_info = NodeInfo {
                    order: order,
                    size: size,
                };
                let node = if character == 'c' {
                    Node::Column(node_info)
                } else {
                    Node::Row(node_info)
                };
                let node_id = self.graph.add_node(node);

                if let Some(id) = parent_id {
                    self.graph.connect(id, node_id);
                }

                let open_bracket_index = end_info_index;
                let close_bracket_index =
                    open_bracket_index + get_closing_brace_index(&target[open_bracket_index..]);
                let mut current_index = open_bracket_index + 1;

                while current_index < close_bracket_index {
                    current_index += self.inner_from_string(
                        &target[current_index..close_bracket_index],
                        Some(node_id),
                    );
                }

                close_bracket_index
            }
            _ => 1, // some other character like a comma that can be skipped
        }
    }
}

#[cfg(test)]
mod tests;
