# Ratatui BarChart, Sparkline, and Gauge Widgets - Complete Guide

## Overview

Ratatui provides several specialized widgets for data visualization that developers often overlook, opting instead to build custom visualizations. These built-in widgets handle the complexity of rendering, scaling, and styling, making them much easier to use than custom implementations.

## BarChart Widget

The `BarChart` widget displays multiple datasets as vertical or horizontal bars with optional grouping and rich styling.

### Basic Usage

```rust
use ratatui::{
    widgets::{Bar, BarChart, BarGroup},
    style::{Style, Color, Stylize},
    Frame,
};

fn render(frame: &mut Frame, area: Rect) {
    // Simple data as tuples
    let data = &[
        ("Mon", 64),
        ("Tue", 72),
        ("Wed", 68),
        ("Thu", 75),
        ("Fri", 82),
    ];
    
    let barchart = BarChart::default()
        .data(data)
        .bar_width(3)
        .bar_gap(1);
    
    frame.render_widget(barchart, area);
}
```

### Bar Creation

There are multiple ways to create bars:

```rust
// Method 1: Simple tuples
let data = &[("Label", 42), ("Other", 58)];
let chart = BarChart::default().data(data);

// Method 2: Explicit Bar construction
let bars = vec![
    Bar::default()
        .value(42)
        .label("Label".into())
        .style(Style::default().fg(Color::Blue)),
    Bar::default()
        .value(58)
        .label("Other".into())
        .style(Style::default().fg(Color::Green)),
];
let group = BarGroup::default().bars(&bars);
let chart = BarChart::default().data(group);

// Method 3: Using builder pattern
let bar = Bar::default()
    .label("Temperature".into())
    .value(72)
    .text_value("72°F".into())
    .style(Style::default().cyan())
    .value_style(Style::default().bold().on_black());
```

### Bar Properties

```rust
let bar = Bar::default()
    // The numeric value (required)
    .value(100)
    
    // Label shown below the bar
    .label("Label".into())
    
    // Custom text shown on the bar (instead of the numeric value)
    .text_value("100%".into())
    
    // Style for the bar itself
    .style(Style::default().fg(Color::Blue))
    
    // Style for the value text on the bar
    .value_style(Style::default().fg(Color::White).bold());
```

### Bar Groups

Group related bars together:

```rust
// Create multiple groups
let group1_bars = vec![
    Bar::default().label("Q1".into()).value(100),
    Bar::default().label("Q2".into()).value(120),
    Bar::default().label("Q3".into()).value(110),
    Bar::default().label("Q4".into()).value(130),
];

let group2_bars = vec![
    Bar::default().label("Q1".into()).value(90),
    Bar::default().label("Q2".into()).value(100),
    Bar::default().label("Q3".into()).value(95),
    Bar::default().label("Q4".into()).value(105),
];

let group1 = BarGroup::default()
    .label("Product A".into())
    .bars(&group1_bars);

let group2 = BarGroup::default()
    .label("Product B".into())
    .bars(&group2_bars);

let chart = BarChart::default()
    .data(group1)
    .data(group2)  // Can call data() multiple times
    .bar_width(3)
    .group_gap(2);
```

### Chart Configuration

```rust
let chart = BarChart::default()
    // Add data
    .data(&[("A", 10), ("B", 20)])
    
    // Set bar appearance
    .bar_width(5)        // Width of each bar
    .bar_gap(2)          // Gap between bars in same group
    .group_gap(3)        // Gap between groups
    
    // Set maximum value (auto-calculated if not set)
    .max(100)
    
    // Overall bar style
    .bar_style(Style::default().fg(Color::Blue))
    
    // Overall value text style
    .value_style(Style::default().fg(Color::Yellow).bold())
    
    // Overall label style
    .label_style(Style::default().fg(Color::White))
    
    // Orientation (default is Vertical)
    .direction(Direction::Vertical)   // or Direction::Horizontal
    
    // Add a block
    .block(Block::bordered().title("Sales Data"));
```

### Horizontal BarChart

```rust
let data = vec![
    Bar::default()
        .text_value("GPU: 75%".into())
        .value(75),
    Bar::default()
        .text_value("CPU: 60%".into())
        .value(60),
    Bar::default()
        .text_value("RAM: 45%".into())
        .value(45),
];

let group = BarGroup::default().bars(&data);

let chart = BarChart::default()
    .data(group)
    .direction(Direction::Horizontal)
    .bar_width(2)
    .bar_gap(1)
    .bar_style(Style::default().fg(Color::Cyan))
    .value_style(Style::default().bg(Color::Blue));
```

### Dynamic Data

```rust
struct MetricsApp {
    values: VecDeque<(String, u64)>,
}

impl MetricsApp {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let bars: Vec<Bar> = self.values
            .iter()
            .enumerate()
            .map(|(i, (label, value))| {
                let style = if *value > 80 {
                    Style::default().fg(Color::Red)
                } else if *value > 60 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::Green)
                };
                
                Bar::default()
                    .label(label.as_str().into())
                    .value(*value)
                    .text_value(format!("{}", value).into())
                    .style(style)
                    .value_style(Style::default().bold())
            })
            .collect();
        
        let group = BarGroup::default().bars(&bars);
        let chart = BarChart::default()
            .data(group)
            .bar_width(3)
            .bar_gap(1)
            .max(100)
            .block(Block::bordered().title("Live Metrics"));
        
        frame.render_widget(chart, area);
    }
    
    fn update(&mut self, label: String, value: u64) {
        // Keep only last N values
        if self.values.len() >= 10 {
            self.values.pop_front();
        }
        self.values.push_back((label, value));
    }
}
```

### Styling Individual Bars

```rust
fn temperature_chart(temps: &[(String, i32)]) -> BarChart {
    let bars: Vec<Bar> = temps
        .iter()
        .map(|(day, temp)| {
            let (style, value_style) = if *temp > 70 {
                (
                    Style::default().fg(Color::Red),
                    Style::default().fg(Color::Gray).bg(Color::Red).bold()
                )
            } else {
                (
                    Style::default().fg(Color::Yellow),
                    Style::default().fg(Color::DarkGray).bg(Color::Yellow).bold()
                )
            };
            
            Bar::default()
                .value(*temp as u64)
                .text_value(format!("{}°", temp).into())
                .label(day.as_str().into())
                .style(style)
                .value_style(value_style)
        })
        .collect();
    
    let group = BarGroup::default().bars(&bars);
    BarChart::default()
        .data(group)
        .bar_width(3)
        .block(Block::bordered().title("Weekly Temperature"))
}
```

## Sparkline Widget

The `Sparkline` widget displays a compact line chart showing trends in a small space - perfect for dashboards and status displays.

### Basic Usage

```rust
use ratatui::widgets::Sparkline;

fn render(frame: &mut Frame, area: Rect) {
    let data = &[0, 2, 3, 4, 1, 4, 10, 8, 7, 9, 10, 8, 6, 4];
    
    let sparkline = Sparkline::default()
        .data(data)
        .style(Style::default().fg(Color::Cyan));
    
    frame.render_widget(sparkline, area);
}
```

### Configuration

```rust
let sparkline = Sparkline::default()
    // Set data points
    .data(&[1, 3, 5, 7, 5, 3, 1])
    
    // Overall style
    .style(Style::default().fg(Color::Green))
    
    // Maximum value (auto-calculated if not set)
    .max(10)
    
    // Direction (default is LeftToRight)
    .direction(Direction::LeftToRight)  // or Direction::RightToLeft
    
    // Add a block
    .block(Block::bordered().title("Trend"));
```

### Dashboard Example

```rust
struct Dashboard {
    cpu_history: VecDeque<u64>,
    memory_history: VecDeque<u64>,
    network_history: VecDeque<u64>,
}

impl Dashboard {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);
        
        // CPU sparkline
        let cpu_data: Vec<u64> = self.cpu_history.iter().copied().collect();
        let cpu = Sparkline::default()
            .data(&cpu_data)
            .style(Style::default().fg(Color::Cyan))
            .max(100)
            .block(Block::bordered().title("CPU Usage"));
        frame.render_widget(cpu, chunks[0]);
        
        // Memory sparkline
        let mem_data: Vec<u64> = self.memory_history.iter().copied().collect();
        let memory = Sparkline::default()
            .data(&mem_data)
            .style(Style::default().fg(Color::Yellow))
            .max(100)
            .block(Block::bordered().title("Memory Usage"));
        frame.render_widget(memory, chunks[1]);
        
        // Network sparkline
        let net_data: Vec<u64> = self.network_history.iter().copied().collect();
        let network = Sparkline::default()
            .data(&net_data)
            .style(Style::default().fg(Color::Green))
            .block(Block::bordered().title("Network (KB/s)"));
        frame.render_widget(network, chunks[2]);
    }
    
    fn update(&mut self, cpu: u64, memory: u64, network: u64) {
        // Keep only last 50 data points
        if self.cpu_history.len() >= 50 {
            self.cpu_history.pop_front();
            self.memory_history.pop_front();
            self.network_history.pop_front();
        }
        
        self.cpu_history.push_back(cpu);
        self.memory_history.push_back(memory);
        self.network_history.push_back(network);
    }
}
```

### Right-to-Left (Latest Data on Right)

```rust
// Show most recent data on the right side
let sparkline = Sparkline::default()
    .data(&recent_values)
    .direction(Direction::RightToLeft)
    .style(Style::default().fg(Color::Magenta));
```

### Multi-line Sparklines

Sparklines can span multiple lines for more detail:

```rust
// Render in a 3-line area for more vertical resolution
let area = Rect { height: 3, ..area };
let sparkline = Sparkline::default()
    .data(&detailed_data)
    .style(Style::default().fg(Color::Blue));
frame.render_widget(sparkline, area);
```

## Gauge Widget

The `Gauge` widget displays progress or percentage values using block characters.

### Basic Usage

```rust
use ratatui::widgets::Gauge;

fn render(frame: &mut Frame, area: Rect) {
    let gauge = Gauge::default()
        .percent(65)
        .label("Progress");
    
    frame.render_widget(gauge, area);
}
```

### Configuration

```rust
let gauge = Gauge::default()
    // Set percentage (0-100)
    .percent(75)
    
    // Or set ratio (0.0-1.0)
    .ratio(0.75)
    
    // Label displayed in the center
    .label("75%")
    
    // Gauge color
    .gauge_style(Style::default().fg(Color::Cyan))
    
    // Use unicode block characters for smoother appearance
    .use_unicode(true)
    
    // Add a block
    .block(Block::bordered().title("Download Progress"));
```

### Styled Progress

```rust
fn progress_gauge(progress: f64, total: f64) -> Gauge<'static> {
    let ratio = progress / total;
    let percent = (ratio * 100.0) as u16;
    
    let style = if percent >= 100 {
        Style::default().fg(Color::Green)
    } else if percent >= 75 {
        Style::default().fg(Color::Yellow)
    } else if percent >= 50 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Red)
    };
    
    Gauge::default()
        .percent(percent)
        .label(format!("{:.1}%", percent as f64))
        .gauge_style(style)
        .use_unicode(true)
}
```

### Custom Labels

```rust
// Absolute values
let gauge = Gauge::default()
    .ratio(0.67)
    .label(format!("{} / {} MB", 670, 1000))
    .gauge_style(Style::default().fg(Color::Magenta));

// Time remaining
let gauge = Gauge::default()
    .percent(45)
    .label("5m 30s remaining")
    .gauge_style(Style::default().fg(Color::Blue));

// Custom styled label
let label = Span::styled(
    format!("{}%", percent),
    Style::default().bold().fg(Color::Black)
);
let gauge = Gauge::default()
    .percent(percent)
    .label(label)
    .gauge_style(Style::default().fg(Color::Yellow));
```

### Multiple Gauges Dashboard

```rust
struct ProgressDashboard {
    cpu: u8,
    memory: u8,
    disk: u8,
    network: u8,
}

impl ProgressDashboard {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);
        
        // CPU Gauge
        let cpu = Gauge::default()
            .percent(self.cpu.into())
            .label(format!("CPU: {}%", self.cpu))
            .gauge_style(self.style_for_percent(self.cpu))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(cpu, chunks[0]);
        
        // Memory Gauge
        let memory = Gauge::default()
            .percent(self.memory.into())
            .label(format!("Memory: {}%", self.memory))
            .gauge_style(self.style_for_percent(self.memory))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(memory, chunks[1]);
        
        // Disk Gauge
        let disk = Gauge::default()
            .percent(self.disk.into())
            .label(format!("Disk: {}%", self.disk))
            .gauge_style(self.style_for_percent(self.disk))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(disk, chunks[2]);
        
        // Network Gauge
        let network = Gauge::default()
            .percent(self.network.into())
            .label(format!("Network: {}%", self.network))
            .gauge_style(self.style_for_percent(self.network))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(network, chunks[3]);
    }
    
    fn style_for_percent(&self, percent: u8) -> Style {
        match percent {
            0..=50 => Style::default().fg(Color::Green),
            51..=75 => Style::default().fg(Color::Yellow),
            76..=90 => Style::default().fg(Color::LightRed),
            _ => Style::default().fg(Color::Red),
        }
    }
}
```

## LineGauge Widget

The `LineGauge` widget is a compact, single-line version of Gauge.

### Basic Usage

```rust
use ratatui::widgets::LineGauge;

let gauge = LineGauge::default()
    .ratio(0.65)
    .label("Progress")
    .style(Style::default().fg(Color::White))
    .gauge_style(Style::default().fg(Color::Cyan))
    .line_set(symbols::line::THICK);

frame.render_widget(gauge, area);
```

### Configuration

```rust
let gauge = LineGauge::default()
    // Progress value
    .ratio(0.75)
    
    // Label
    .label("75%")
    
    // Gauge bar color
    .gauge_style(Style::default().fg(Color::Green))
    
    // Overall style
    .style(Style::default().fg(Color::White))
    
    // Line characters (NORMAL, THICK, DOUBLE)
    .line_set(symbols::line::THICK);
```

### Compact Status Display

```rust
fn compact_status(frame: &mut Frame, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    
    // Very space-efficient status display
    let cpu = LineGauge::default()
        .ratio(0.67)
        .label("CPU: 67%")
        .gauge_style(Style::default().fg(Color::Cyan))
        .line_set(symbols::line::THICK);
    frame.render_widget(cpu, chunks[0]);
    
    let mem = LineGauge::default()
        .ratio(0.82)
        .label("MEM: 82%")
        .gauge_style(Style::default().fg(Color::Yellow))
        .line_set(symbols::line::THICK);
    frame.render_widget(mem, chunks[1]);
    
    let disk = LineGauge::default()
        .ratio(0.45)
        .label("DSK: 45%")
        .gauge_style(Style::default().fg(Color::Green))
        .line_set(symbols::line::THICK);
    frame.render_widget(disk, chunks[2]);
}
```

## Practical Examples

### System Monitor Dashboard

```rust
struct SystemMonitor {
    cpu_percent: u8,
    cpu_history: VecDeque<u64>,
    memory_gb: f64,
    memory_total_gb: f64,
    disk_percent: u8,
    processes: Vec<(String, u64)>,
}

impl SystemMonitor {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(3),  // CPU gauge
            Constraint::Length(3),  // CPU sparkline
            Constraint::Length(3),  // Memory gauge
            Constraint::Length(3),  // Disk gauge
            Constraint::Min(0),     // Process bar chart
        ])
        .split(area);
        
        // CPU Gauge
        let cpu_gauge = Gauge::default()
            .percent(self.cpu_percent.into())
            .label(format!("CPU: {}%", self.cpu_percent))
            .gauge_style(Style::default().fg(Color::Cyan))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(cpu_gauge, chunks[0]);
        
        // CPU History Sparkline
        let cpu_data: Vec<u64> = self.cpu_history.iter().copied().collect();
        let cpu_spark = Sparkline::default()
            .data(&cpu_data)
            .style(Style::default().fg(Color::Cyan))
            .max(100)
            .block(Block::bordered().title("CPU History"));
        frame.render_widget(cpu_spark, chunks[1]);
        
        // Memory Gauge
        let mem_ratio = self.memory_gb / self.memory_total_gb;
        let memory = Gauge::default()
            .ratio(mem_ratio)
            .label(format!("{:.1} / {:.1} GB", self.memory_gb, self.memory_total_gb))
            .gauge_style(Style::default().fg(Color::Yellow))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(memory, chunks[2]);
        
        // Disk Gauge
        let disk = Gauge::default()
            .percent(self.disk_percent.into())
            .label(format!("Disk: {}%", self.disk_percent))
            .gauge_style(Style::default().fg(Color::Magenta))
            .use_unicode(true)
            .block(Block::bordered());
        frame.render_widget(disk, chunks[3]);
        
        // Top Processes Bar Chart
        let bars: Vec<Bar> = self.processes
            .iter()
            .map(|(name, cpu)| {
                Bar::default()
                    .label(name.as_str().into())
                    .value(*cpu)
                    .text_value(format!("{}%", cpu).into())
                    .style(Style::default().fg(Color::Green))
            })
            .collect();
        
        let group = BarGroup::default().bars(&bars);
        let processes = BarChart::default()
            .data(group)
            .bar_width(3)
            .bar_gap(1)
            .max(100)
            .block(Block::bordered().title("Top Processes"));
        frame.render_widget(processes, chunks[4]);
    }
}
```

### Download Progress Tracker

```rust
struct DownloadTracker {
    downloads: Vec<Download>,
}

struct Download {
    name: String,
    progress: u64,
    total: u64,
    speed_kbps: u64,
}

impl DownloadTracker {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunk_size = 4; // 3 lines for gauge + 1 for spacing
        let chunks = Layout::vertical(
            std::iter::repeat(Constraint::Length(chunk_size as u16))
                .take(self.downloads.len())
                .collect::<Vec<_>>()
        )
        .split(area);
        
        for (i, download) in self.downloads.iter().enumerate() {
            let percent = ((download.progress as f64 / download.total as f64) * 100.0) as u16;
            let ratio = download.progress as f64 / download.total as f64;
            
            let label = format!(
                "{} / {} MB @ {} KB/s",
                download.progress / 1024 / 1024,
                download.total / 1024 / 1024,
                download.speed_kbps
            );
            
            let gauge = Gauge::default()
                .percent(percent)
                .label(label)
                .gauge_style(
                    if percent >= 100 {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Cyan)
                    }
                )
                .use_unicode(true)
                .block(Block::bordered().title(download.name.clone()));
            
            frame.render_widget(gauge, chunks[i]);
        }
    }
}
```

## Common Pitfalls

### ❌ Not Setting max for BarChart

```rust
// Wrong - bars auto-scale, making comparison difficult
let chart1 = BarChart::default().data(&[("A", 10), ("B", 20)]);
let chart2 = BarChart::default().data(&[("A", 100), ("B", 200)]);
// Both look the same height!
```

```rust
// Right - set consistent max
let max_value = 200;
let chart1 = BarChart::default().data(&[("A", 10), ("B", 20)]).max(max_value);
let chart2 = BarChart::default().data(&[("A", 100), ("B", 200)]).max(max_value);
```

### ❌ Using percent() for non-percentage values

```rust
// Wrong - Gauge expects 0-100 for percent()
let gauge = Gauge::default().percent(0.75);  // Will show 0%!
```

```rust
// Right - use ratio() for decimal values
let gauge = Gauge::default().ratio(0.75);    // 75%
// Or convert to percent
let gauge = Gauge::default().percent(75);    // 75%
```

### ❌ Forgetting to collect Sparkline data

```rust
// Wrong - Sparkline needs &[u64]
let history: VecDeque<u64> = ...;
let sparkline = Sparkline::default().data(&history);  // Won't compile
```

```rust
// Right - convert to Vec
let data: Vec<u64> = history.iter().copied().collect();
let sparkline = Sparkline::default().data(&data);
```

## When to Use Which Widget

**Use BarChart when:**
- Comparing discrete categories
- Showing grouped data
- Need labels and precise values
- Have space for detailed visualization

**Use Sparkline when:**
- Showing trends in minimal space
- Dashboard indicators
- Time series in compact form
- Background monitoring

**Use Gauge when:**
- Showing single progress value
- Need visual percentage indicator
- Status displays
- Resource usage (CPU, memory, disk)

**Use LineGauge when:**
- Very limited vertical space
- Multiple progress bars in small area
- Compact status displays
- Terminal or sidebar widgets

## Further Reading

- [BarChart API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.BarChart.html)
- [Sparkline API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Sparkline.html)
- [Gauge API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Gauge.html)
- [LineGauge API](https://docs.rs/ratatui/latest/ratatui/widgets/struct.LineGauge.html)
- [BarChart Example](https://ratatui.rs/examples/widgets/barchart/)
- [Sparkline Example](https://ratatui.rs/examples/widgets/sparkline/)
- [Gauge Example](https://ratatui.rs/examples/widgets/gauge/)
