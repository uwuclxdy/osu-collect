# Ratatui Stateful Widgets - Complete Guide

## Overview

Stateful widgets in Ratatui are widgets that maintain state between render cycles. This state is separate from your application's data and handles UI-specific concerns like selection, scrolling, and viewport positioning. Understanding and using these built-in state types can eliminate tons of custom index tracking and bounds checking code.

## The StatefulWidget Trait

The `StatefulWidget` trait is the foundation for widgets that require state management:

```rust
pub trait StatefulWidget {
    type State;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State);
}
```

Key widgets that implement `StatefulWidget`:
- `List` with `ListState`
- `Table` with `TableState`
- `Scrollbar` with `ScrollbarState`

## Core State Types

### 1. ListState

`ListState` manages selection and scrolling for `List` widgets.

#### Basic Usage

```rust
use ratatui::{
    widgets::{List, ListItem, ListState},
    Frame,
};

struct App {
    list_state: ListState,
    items: Vec<String>,
}

impl App {
    fn new(items: Vec<String>) -> Self {
        Self {
            list_state: ListState::default(),
            items,
        }
    }
    
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.items
            .iter()
            .map(|s| ListItem::new(s.as_str()))
            .collect();
        
        let list = List::new(items)
            .highlight_style(Style::default().bg(Color::Blue))
            .highlight_symbol(">> ");
        
        // Note: render_stateful_widget, not render_widget
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}
```

#### ListState Methods

**Selection Management:**
```rust
// Select specific index
list_state.select(Some(5));

// Get current selection
if let Some(i) = list_state.selected() {
    println!("Selected: {}", i);
}

// Unselect (nothing highlighted)
list_state.select(None);

// Direct mutable access
*list_state.selected_mut() = Some(10);
```

**Navigation Methods (added in recent versions):**
```rust
// Move to next item (wraps to 0 at end)
list_state.select_next();

// Move to previous item (wraps to last at beginning)
list_state.select_previous();

// Jump to specific positions
list_state.select_first();
list_state.select_last();

// Scroll by amount
list_state.scroll_down_by(5);
list_state.scroll_up_by(3);
```

**Important:** Navigation methods handle bounds automatically and work even before the list is rendered.

#### Managing Offset

ListState automatically manages the viewport offset to keep the selected item visible:

```rust
// The offset is internal and calculated automatically
// You typically don't access it directly, but it's there

// When you select an item outside the viewport:
list_state.select(Some(100)); // Even if only 20 items visible

// On next render, ListState automatically:
// 1. Calculates the viewport size
// 2. Sets offset to show the selected item
// 3. Implements smooth "natural" scrolling
```

#### Common Pattern: Wrapper Struct

```rust
struct StatefulList {
    state: ListState,
    items: Vec<String>,
}

impl StatefulList {
    fn new(items: Vec<String>) -> Self {
        Self {
            state: ListState::default(),
            items,
        }
    }
    
    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }
    
    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }
    
    fn unselect(&mut self) {
        self.state.select(None);
    }
}

// Usage in event handler
match key.code {
    KeyCode::Down => app.list.next(),
    KeyCode::Up => app.list.previous(),
    KeyCode::Esc => app.list.unselect(),
    _ => {}
}
```

### 2. TableState

`TableState` manages selection and scrolling for `Table` widgets.

#### Basic Usage

```rust
use ratatui::{
    layout::Constraint,
    widgets::{Row, Table, TableState},
    Frame,
};

struct App {
    table_state: TableState,
    rows: Vec<Vec<String>>,
}

impl App {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self.rows
            .iter()
            .map(|row| Row::new(row.clone()))
            .collect();
        
        let widths = [
            Constraint::Length(15),
            Constraint::Length(20),
            Constraint::Min(10),
        ];
        
        let table = Table::new(rows, widths)
            .header(Row::new(vec!["Col1", "Col2", "Col3"])
                .style(Style::default().bold()))
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol(">> ");
        
        frame.render_stateful_widget(table, area, &mut self.table_state);
    }
}
```

#### TableState Methods

TableState has similar methods to ListState:

```rust
// Selection
table_state.select(Some(3));
let selected = table_state.selected();
table_state.select(None);

// Navigation (same as ListState)
table_state.select_next();
table_state.select_previous();
table_state.select_first();
table_state.select_last();
table_state.scroll_down_by(5);
table_state.scroll_up_by(3);

// Builder pattern
let state = TableState::default()
    .with_selected(Some(5));
```

#### Column Selection (if needed)

While TableState handles row selection, column selection requires custom tracking:

```rust
struct TableApp {
    row_state: TableState,
    selected_column: usize,
}

// Handle column navigation separately
match key.code {
    KeyCode::Left => {
        if app.selected_column > 0 {
            app.selected_column -= 1;
        }
    }
    KeyCode::Right => {
        if app.selected_column < num_columns - 1 {
            app.selected_column += 1;
        }
    }
    _ => {}
}
```

### 3. ScrollbarState

`ScrollbarState` manages scrollbar position and rendering for scrollable content.

#### Basic Concepts

ScrollbarState tracks:
- **content_length**: Total number of items/lines
- **position**: Current scroll position (which item is at top)
- **viewport_content_length**: How many items visible (optional)

#### Basic Usage with List

```rust
use ratatui::{
    layout::Margin,
    widgets::{List, ListState, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

struct App {
    list_state: ListState,
    items: Vec<String>,
}

impl App {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Render the list
        let items: Vec<ListItem> = self.items
            .iter()
            .map(|s| ListItem::new(s.as_str()))
            .collect();
        
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL));
        
        frame.render_stateful_widget(list, area, &mut self.list_state);
        
        // Add scrollbar
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        
        let mut scrollbar_state = ScrollbarState::new(self.items.len())
            .position(self.list_state.selected().unwrap_or(0));
        
        // Render scrollbar with margin to place it inside the border
        let scrollbar_area = area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        });
        
        frame.render_stateful_widget(
            scrollbar,
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}
```

#### ScrollbarState Methods

```rust
// Create with content length (required!)
let mut state = ScrollbarState::new(100); // 100 items total

// Builder pattern
let state = ScrollbarState::default()
    .content_length(100)
    .position(25);

// Update position
state = state.position(current_scroll);

// Update content length when data changes
state = state.content_length(new_item_count);

// For multi-line items
state = state
    .content_length(100)  // Total items
    .viewport_content_length(25);  // Visible lines
```

#### Scrollbar with Table

```rust
fn render_table_with_scrollbar(
    frame: &mut Frame,
    area: Rect,
    table_state: &mut TableState,
    row_count: usize,
) {
    // Render table
    let table = Table::new(rows, widths)
        .block(Block::default().borders(Borders::ALL));
    
    frame.render_stateful_widget(table, area, table_state);
    
    // Scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    let mut scrollbar_state = ScrollbarState::new(row_count)
        .position(table_state.selected().unwrap_or(0));
    
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut scrollbar_state,
    );
}
```

#### Scrollbar with Paragraph

```rust
struct TextViewer {
    vertical_scroll: usize,
    content: Vec<Line<'static>>,
}

impl TextViewer {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(self.content.clone())
            .scroll((self.vertical_scroll as u16, 0))
            .block(Block::default().borders(Borders::RIGHT));
        
        frame.render_widget(paragraph, area);
        
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        
        let mut scrollbar_state = ScrollbarState::new(self.content.len())
            .position(self.vertical_scroll);
        
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
    
    fn scroll_down(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_add(1);
        if self.vertical_scroll >= self.content.len() {
            self.vertical_scroll = self.content.len().saturating_sub(1);
        }
    }
    
    fn scroll_up(&mut self) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
    }
}
```

## State Lifecycle

### Persistence Between Renders

**Critical:** State must persist between render calls:

```rust
// ❌ WRONG - State recreated each render
fn render(frame: &mut Frame, area: Rect, items: &[String]) {
    let mut state = ListState::default();  // Lost each frame!
    state.select(Some(0));
    // ... render
}

// ✅ CORRECT - State stored in app struct
struct App {
    list_state: ListState,  // Persists between renders
    items: Vec<String>,
}
```

### State Initialization

```rust
// Default (nothing selected)
let state = ListState::default();

// With initial selection
let state = TableState::default().with_selected(Some(0));

// Initialize with data
struct App {
    items: Vec<String>,
    list_state: ListState,
}

impl App {
    fn new(items: Vec<String>) -> Self {
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));  // Select first item
        }
        Self { items, list_state }
    }
}
```

### Resetting State

```rust
impl App {
    fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        // Reset state when data changes
        self.list_state = ListState::default();
    }
    
    fn refresh(&mut self) {
        // Keep selection if possible
        let selected = self.list_state.selected();
        self.reload_items();
        
        // Restore selection if still valid
        if let Some(i) = selected {
            if i < self.items.len() {
                self.list_state.select(Some(i));
            } else {
                // Select last item if previous selection out of bounds
                self.list_state.select(Some(self.items.len() - 1));
            }
        }
    }
}
```

## Advanced Patterns

### Multi-widget State Coordination

```rust
struct Dashboard {
    sidebar_state: ListState,
    table_state: TableState,
    active_widget: ActiveWidget,
}

enum ActiveWidget {
    Sidebar,
    Table,
}

impl Dashboard {
    fn handle_key(&mut self, key: KeyCode) {
        match self.active_widget {
            ActiveWidget::Sidebar => match key {
                KeyCode::Down => self.sidebar_state.select_next(),
                KeyCode::Up => self.sidebar_state.select_previous(),
                KeyCode::Right => self.active_widget = ActiveWidget::Table,
                _ => {}
            },
            ActiveWidget::Table => match key {
                KeyCode::Down => self.table_state.select_next(),
                KeyCode::Up => self.table_state.select_previous(),
                KeyCode::Left => self.active_widget = ActiveWidget::Sidebar,
                _ => {}
            },
        }
    }
}
```

### Filtered Lists with State

```rust
struct FilteredList {
    items: Vec<String>,
    filtered_indices: Vec<usize>,
    state: ListState,
    filter: String,
}

impl FilteredList {
    fn update_filter(&mut self, filter: String) {
        self.filter = filter;
        self.filtered_indices = self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.contains(&self.filter))
            .map(|(i, _)| i)
            .collect();
        
        // Reset selection to first filtered item
        if !self.filtered_indices.is_empty() {
            self.state.select(Some(0));
        } else {
            self.state.select(None);
        }
    }
    
    fn selected_item(&self) -> Option<&String> {
        self.state.selected()
            .and_then(|i| self.filtered_indices.get(i))
            .and_then(|&idx| self.items.get(idx))
    }
    
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.filtered_indices
            .iter()
            .filter_map(|&i| self.items.get(i))
            .map(|s| ListItem::new(s.as_str()))
            .collect();
        
        let list = List::new(items).highlight_symbol(">> ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }
}
```

### Paginated Table

```rust
struct PaginatedTable {
    state: TableState,
    page_size: usize,
    total_rows: usize,
}

impl PaginatedTable {
    fn current_page(&self) -> usize {
        self.state.selected()
            .map(|sel| sel / self.page_size)
            .unwrap_or(0)
    }
    
    fn total_pages(&self) -> usize {
        (self.total_rows + self.page_size - 1) / self.page_size
    }
    
    fn next_page(&mut self) {
        let next_page_start = (self.current_page() + 1) * self.page_size;
        if next_page_start < self.total_rows {
            self.state.select(Some(next_page_start));
        }
    }
    
    fn prev_page(&mut self) {
        let current_page = self.current_page();
        if current_page > 0 {
            let prev_page_start = (current_page - 1) * self.page_size;
            self.state.select(Some(prev_page_start));
        }
    }
}
```

## Common Pitfalls

### ❌ Not Using Stateful Rendering

```rust
// Wrong - state changes ignored
frame.render_widget(list, area);
```

```rust
// Correct - state applied
frame.render_stateful_widget(list, area, &mut list_state);
```

### ❌ Recreating State Each Frame

```rust
// Wrong - loses selection
fn render(&self, frame: &mut Frame) {
    let mut state = ListState::default();  // Reset every frame!
    // ...
}
```

```rust
// Correct - state persists
struct App {
    state: ListState,  // Stored in app
}
```

### ❌ Not Handling Bounds

```rust
// Wrong - can select out of bounds
self.state.select(Some(user_input));
```

```rust
// Correct - validate bounds
let index = user_input.min(items.len() - 1);
self.state.select(Some(index));

// Or use built-in navigation
self.state.select_next();  // Handles bounds automatically
```

### ❌ Forgetting to Update ScrollbarState

```rust
// Wrong - scrollbar position never changes
let scrollbar_state = ScrollbarState::new(items.len());
```

```rust
// Correct - update position
let mut scrollbar_state = ScrollbarState::new(items.len())
    .position(list_state.selected().unwrap_or(0));
```

## Testing Stateful Widgets

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_list_navigation() {
        let mut list = StatefulList::new(vec![
            "Item 1".to_string(),
            "Item 2".to_string(),
            "Item 3".to_string(),
        ]);
        
        // Initial state
        assert_eq!(list.state.selected(), None);
        
        // Select first
        list.next();
        assert_eq!(list.state.selected(), Some(0));
        
        // Cycle through
        list.next();
        assert_eq!(list.state.selected(), Some(1));
        
        list.next();
        assert_eq!(list.state.selected(), Some(2));
        
        // Wrap around
        list.next();
        assert_eq!(list.state.selected(), Some(0));
        
        // Go backwards
        list.previous();
        assert_eq!(list.state.selected(), Some(2));
    }
    
    #[test]
    fn test_table_state_builder() {
        let state = TableState::default().with_selected(Some(5));
        assert_eq!(state.selected(), Some(5));
    }
}
```

## Performance Considerations

- State objects are lightweight (typically just a few integers)
- State updates don't trigger renders - only change what gets rendered
- Offset calculations are done during render, not when state changes
- Multiple state objects can be stored with minimal overhead

## Migration from Custom State

If you've been managing selection manually:

```rust
// Before
struct App {
    selected_index: Option<usize>,
    scroll_offset: usize,
}

// After
struct App {
    list_state: ListState,
}
```

Benefits:
- Automatic offset management
- Bounds checking built-in
- Natural scrolling behavior
- Consistent API across widgets
- Less code to maintain

## Further Reading

- [StatefulWidget Trait](https://docs.rs/ratatui/latest/ratatui/widgets/trait.StatefulWidget.html)
- [ListState API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.ListState.html)
- [TableState API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.TableState.html)
- [ScrollbarState API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.ScrollbarState.html)
- [Widget Concepts](https://ratatui.rs/concepts/widgets/)
