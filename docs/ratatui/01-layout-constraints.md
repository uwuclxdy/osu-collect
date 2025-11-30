# Ratatui Layout Constraints - Complete Guide

## Overview

Layout constraints are the fundamental building blocks for creating terminal user interfaces in Ratatui. They define how space is divided and allocated to different UI components. Understanding constraints deeply can replace most custom layout logic developers tend to write.

## Core Constraint Types

### 1. `Constraint::Length(u16)`
Fixed absolute size in rows or columns.

```rust
use ratatui::layout::{Constraint, Direction, Layout};

let layout = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(3),  // Header: exactly 3 rows
        Constraint::Length(1),  // Separator: exactly 1 row
    ])
    .split(area);
```

**Key characteristics:**
- Non-responsive to terminal size changes
- Takes exact number of cells specified
- Highest priority in constraint resolution
- Use for fixed UI elements (headers, status bars, borders)

### 2. `Constraint::Percentage(u16)`
Relative size as percentage of parent area.

```rust
let layout = Layout::default()
    .constraints([
        Constraint::Percentage(70),  // 70% of width
        Constraint::Percentage(30),  // 30% of width
    ])
    .split(area);
```

**Key characteristics:**
- Calculated relative to entire available space
- Not affected by other constraints' sizes
- Value must be 0-100
- **Limitation**: Cannot represent ratios like 1/3 exactly (use `Ratio` instead)

### 3. `Constraint::Ratio(u16, u16)`
Fine-grained proportional division using ratios.

```rust
let layout = Layout::default()
    .constraints([
        Constraint::Ratio(1, 3),  // 1/3 of space
        Constraint::Ratio(2, 3),  // 2/3 of space
    ])
    .split(area);
```

**Key characteristics:**
- More precise than Percentage for divisions like 1/3, 2/5, etc.
- First parameter: numerator, second: denominator
- Calculated relative to entire available space
- Can represent exact mathematical ratios

### 4. `Constraint::Min(u16)`
Minimum size with ability to expand.

```rust
let layout = Layout::default()
    .constraints([
        Constraint::Length(10),     // Fixed header
        Constraint::Min(20),        // Main area, at least 20 rows
        Constraint::Length(3),      // Fixed footer
    ])
    .split(area);
```

**Key characteristics:**
- Guarantees minimum size
- Will expand to fill remaining space if available
- Lower priority than Length
- Common for main content areas

### 5. `Constraint::Max(u16)`
Maximum size with ability to shrink.

```rust
let layout = Layout::default()
    .constraints([
        Constraint::Max(40),        // Sidebar, max 40 cols
        Constraint::Min(0),         // Main area, takes remainder
    ])
    .split(area);
```

**Key characteristics:**
- Sets upper bound on size
- Will shrink if not enough space
- Useful for optional/collapsible panels
- Lower priority than Length

### 6. `Constraint::Fill(u16)`
Proportional filling of excess space (added in v0.25.0).

```rust
let layout = Layout::default()
    .constraints([
        Constraint::Length(10),     // Fixed
        Constraint::Fill(1),        // Gets 1 part of remaining
        Constraint::Fill(2),        // Gets 2 parts of remaining
    ])
    .split(area);
```

**Key characteristics:**
- Only expands into **excess** available space
- Acts proportionally among other Fill constraints
- Doesn't affect space needed by other constraint types
- Excellent for flexible layouts with some fixed elements

**Example output** (50 cells total):
```
┌──────────┐  ← Length(10): 10 cells
┌─────────────┐  ← Fill(1): 13 cells
┌──────────────────────────┐  ← Fill(2): 27 cells (twice as much)
```

## Constraint Priority Order

When space is limited, constraints are resolved in this order:

1. **Length** - Fixed sizes allocated first
2. **Max** - Maximum bounds enforced
3. **Min** - Minimum sizes guaranteed
4. **Percentage/Ratio** - Relative sizes calculated
5. **Fill** - Excess space distributed proportionally

## Advanced Layout Patterns

### Nested Layouts

Combine layouts for complex structures:

```rust
fn render(frame: &mut Frame, area: Rect) {
    // Vertical split: header, main, footer
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ]);
    let [header, main, footer] = vertical.areas(area);
    
    // Horizontal split of main area
    let horizontal = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(70),
    ]);
    let [sidebar, content] = horizontal.areas(main);
    
    frame.render_widget(Header::new(), header);
    frame.render_widget(Sidebar::new(), sidebar);
    frame.render_widget(Content::new(), content);
    frame.render_widget(StatusBar::new(), footer);
}
```

### Centered Layouts

Create centered rectangles using `Flex::Center`:

```rust
use ratatui::layout::Flex;

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let horizontal = Layout::horizontal([width]).flex(Flex::Center);
    let vertical = Layout::vertical([height]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}

// Usage
let popup_area = centered_rect(frame.area(), 60, 20);
frame.render_widget(Popup::new(), popup_area);
```

### Flex Layout Options

Control how excess space is distributed:

```rust
use ratatui::layout::Flex;

// Default: Last element takes all remaining space
let layout = Layout::vertical([
    Constraint::Length(5),
    Constraint::Min(0),
]);

// Even distribution of excess space
let layout = Layout::vertical([
    Constraint::Length(5),
    Constraint::Length(5),
]).flex(Flex::SpaceAround);

// Available Flex options:
// - Flex::Start: Align to start
// - Flex::Center: Center alignment
// - Flex::End: Align to end
// - Flex::SpaceBetween: Space between items
// - Flex::SpaceAround: Space around items
```

## Helper Methods

Ratatui provides convenient constructors for common constraint patterns:

### `from_lengths()`
```rust
let constraints = Constraint::from_lengths([10, 20, 10]);
// Equivalent to:
// [Constraint::Length(10), Constraint::Length(20), Constraint::Length(10)]
```

### `from_ratios()`
```rust
let constraints = Constraint::from_ratios([(1, 4), (1, 2), (1, 4)]);
// Creates centered layout: 25% - 50% - 25%
```

### `from_percentages()`
```rust
let constraints = Constraint::from_percentages([25, 50, 25]);
```

### `from_mins()`
```rust
let constraints = Constraint::from_mins([10, 20, 10]);
```

### `from_maxes()`
```rust
let constraints = Constraint::from_maxes([40, 80, 40]);
```

## Modern API Patterns

### Array-based constraints (no `vec![]` needed)
```rust
// Old style (still works)
Layout::default()
    .constraints(vec![Constraint::Length(5), Constraint::Min(0)])
    .split(area)

// New style - cleaner
Layout::vertical([
    Constraint::Length(5),
    Constraint::Min(0),
])
.split(area)
```

### Type inference for u16
```rust
// Implicit conversion from u16 to Length
Layout::horizontal([1, 2, 3])
    .split(area)

// Equivalent to:
Layout::horizontal([
    Constraint::Length(1),
    Constraint::Length(2),
    Constraint::Length(3),
])
.split(area)
```

### Compile-time known constraints with `areas()`
```rust
// When you know constraint count at compile time, use areas()
let [top, main, bottom] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(1),
])
.areas(area);

// No indexing needed, destructure directly
frame.render_widget(title, top);
frame.render_widget(content, main);
frame.render_widget(status, bottom);
```

### Runtime constraints with `split()`
```rust
// When constraints are dynamic
let constraints: Vec<Constraint> = items
    .iter()
    .map(|item| Constraint::Length(item.height))
    .collect();

let areas = Layout::vertical(constraints).split(area);

for (widget, area) in widgets.iter().zip(areas.iter()) {
    frame.render_widget(widget, *area);
}
```

## Common Pitfalls and Solutions

### ❌ Problem: Last element takes all remaining space
```rust
// This gives all excess space to the last element
Layout::vertical([
    Constraint::Length(5),
    Constraint::Length(5),
])
```

### ✅ Solution: Add `Min(0)` as last constraint
```rust
Layout::vertical([
    Constraint::Length(5),
    Constraint::Length(5),
    Constraint::Min(0),  // Absorbs excess space
])
```

### ❌ Problem: Percentage doesn't account for fixed elements
```rust
// Both try to use full space, conflict occurs
Layout::horizontal([
    Constraint::Length(20),      // Takes 20
    Constraint::Percentage(100), // Wants all space
])
```

### ✅ Solution: Use Fill or nested layouts
```rust
// Option 1: Use Fill for proportional remaining space
Layout::horizontal([
    Constraint::Length(20),
    Constraint::Fill(1),
])

// Option 2: Calculate percentage of remaining
// Better to nest layouts for clarity
let [fixed, remaining] = Layout::horizontal([
    Constraint::Length(20),
    Constraint::Min(0),
]).areas(area);
```

### ❌ Problem: Complex layout in single split
```rust
// Hard to read and maintain
Layout::default()
    .constraints([
        Constraint::Length(1),
        Constraint::Percentage(20),
        Constraint::Min(10),
        Constraint::Max(30),
        Constraint::Length(3),
    ])
```

### ✅ Solution: Use nested layouts
```rust
// Split into logical sections
let [header, main, footer] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(3),
]).areas(area);

// Further split main area
let [sidebar, content] = Layout::horizontal([
    Constraint::Percentage(20),
    Constraint::Min(0),
]).areas(main);
```

## Performance Considerations

### Layout Caching

Ratatui caches layout calculations in a thread-local LRU cache:

```rust
use ratatui::layout::Layout;

// Configure cache size (default is reasonable for most apps)
Layout::init_cache(100);

// Layouts with same parameters hit cache
let layout = Layout::vertical([Constraint::Length(5), Constraint::Min(0)]);
let areas1 = layout.split(rect1);  // Calculated
let areas2 = layout.split(rect1);  // Cached (same rect)
let areas3 = layout.split(rect2);  // Calculated (different rect)
```

### Constraint Solver

Ratatui uses the **Cassowary** constraint solving algorithm:
- Handles conflicting constraints gracefully
- Results may be non-deterministic when constraints cannot all be satisfied
- Generally very fast for typical TUI layouts
- Over-constraining (making impossible demands) leads to "best effort" solutions

## Real-World Examples

### Dashboard Layout
```rust
fn dashboard_layout(area: Rect) -> [Rect; 6] {
    let vertical = Layout::vertical([
        Constraint::Length(3),      // Title bar
        Constraint::Min(10),        // Main content
        Constraint::Length(1),      // Status bar
    ]);
    let [title, main, status] = vertical.areas(area);
    
    let main_split = Layout::horizontal([
        Constraint::Percentage(25), // Sidebar
        Constraint::Percentage(75), // Content
    ]);
    let [sidebar, content] = main_split.areas(main);
    
    let content_split = Layout::vertical([
        Constraint::Ratio(1, 2),    // Top panels
        Constraint::Ratio(1, 2),    // Bottom panels
    ]);
    let [top, bottom] = content_split.areas(content);
    
    let panels = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Fill(1),
    ]);
    let [top_left, top_right] = panels.areas(top);
    let [bottom_left, bottom_right] = panels.areas(bottom);
    
    [title, sidebar, top_left, top_right, bottom_left, bottom_right]
}
```

### Responsive Layout
```rust
fn responsive_layout(area: Rect) -> Vec<Rect> {
    let min_width_for_sidebar = 80;
    
    if area.width >= min_width_for_sidebar {
        // Wide screen: show sidebar
        Layout::horizontal([
            Constraint::Length(20),
            Constraint::Min(0),
        ])
        .split(area)
        .to_vec()
    } else {
        // Narrow screen: full width
        vec![area]
    }
}
```

### Equal Spacing with Fill
```rust
// Create 4 equal columns that share space
let columns = Layout::horizontal([
    Constraint::Fill(1),
    Constraint::Fill(1),
    Constraint::Fill(1),
    Constraint::Fill(1),
])
.split(area);

// Or more concisely with array repeat
let columns = Layout::horizontal([Constraint::Fill(1); 4])
    .split(area);
```

## Testing Layouts

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::{Constraint, Layout, Rect};
    
    #[test]
    fn test_layout_split() {
        let area = Rect::new(0, 0, 100, 50);
        let [top, bottom] = Layout::vertical([
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .areas(area);
        
        assert_eq!(top.height, 10);
        assert_eq!(bottom.height, 40);
        assert_eq!(top.y, 0);
        assert_eq!(bottom.y, 10);
    }
}
```

## Migration Tips

If you've been writing custom layout logic:

1. **Replace manual calculations** with constraint combinations
2. **Use `areas()` instead of indexing** for known sizes
3. **Leverage `Fill`** instead of complex remainder calculations
4. **Nest layouts** instead of trying to do everything in one split
5. **Use helper methods** like `from_ratios()` for cleaner code

## Further Reading

- [Official Layout Documentation](https://ratatui.rs/concepts/layout/)
- [Constraint API Docs](https://docs.rs/ratatui/latest/ratatui/layout/enum.Constraint.html)
- [Constraint Explorer Example](https://ratatui.rs/examples/layout/constraint-explorer/)
- [Flex Layout Guide](https://docs.rs/ratatui/latest/ratatui/layout/enum.Flex.html)
