//! CAM: the machining setup a part is cut in, the tool library it is cut with, and the
//! toolpaths that cut it.
//!
//! A [`CoordinateSystem`] places the machine zero and the machine's own X, Y and Z directions
//! (`cam.coordinate_system.create`); a [`Setup`] takes that origin and names the raw stock, a
//! [`Tool`] is one cutter of the design's library, a [`Toolpath`] is one operation (facing,
//! contouring or pocketing) of a setup, cut with one tool, and a [`CamContour2d`] is a profile
//! operation that follows edges the user picked.
//!
//! Nothing here is geometry: the operations name the bodies and the coordinate system they work
//! from, and the engine reads the part's exact B-rep when it generates a path. Setups, tools and
//! toolpaths share the design's item ids with everything else, so a browser card, a pick and a
//! command all mean the same object by one number.

use serde::{Deserialize, Serialize};

/// A machining origin with the machine's own axes: the machine zero a toolpath works from.
///
/// `origin` is in model space (mm) and the axes are unit vectors in model space, so moving or
/// rotating the model carries the coordinate system with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSystem {
    /// Its identity in the design; never reused.
    #[serde(default)]
    pub id: u64,
    /// Its name in the browser and in a toolpath's program.
    #[serde(default)]
    pub name: String,
    /// Where the machine zero sits, in mm.
    #[serde(default)]
    pub origin: [f64; 3],
    /// The machine's +X direction: a unit vector.
    #[serde(default = "model_x")]
    pub x_axis: [f64; 3],
    /// The machine's +Y direction: a unit vector.
    #[serde(default = "model_y")]
    pub y_axis: [f64; 3],
    /// The machine's +Z direction: a unit vector.
    #[serde(default = "model_z")]
    pub z_axis: [f64; 3],
}

impl Default for CoordinateSystem {
    fn default() -> Self {
        CoordinateSystem { id: 0, name: String::new(), origin: [0.0; 3], x_axis: model_x(), y_axis: model_y(), z_axis: model_z() }
    }
}

/// The design's CAM data: its setups, its tool library and the toolpaths that run in the setups.
///
/// A design without CAM data saves without the key and opens as an empty set, as it did before
/// CAM existed (`crates/doc/src/format/pending/zcam-toolpaths.rs`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CamSettings {
    /// How the part is held and where it is machined from.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub setups: Vec<Setup>,
    /// The cutters a toolpath may use.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    /// The operations, each in one setup and on one tool.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub toolpaths: Vec<Toolpath>,
    /// The 2D contour operations, each in one setup and on one tool.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contours: Vec<CamContour2d>,
}

impl CamSettings {
    /// Whether the design keeps no CAM data at all (it then saves without the `cam` key).
    pub fn is_empty(&self) -> bool {
        self.setups.is_empty() && self.tools.is_empty() && self.toolpaths.is_empty() && self.contours.is_empty()
    }

    /// Every identity these keep: setups, tools, toolpaths and contours take ids from the
    /// design's one counter, as every other item does.
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        let setups = self.setups.iter().map(|s| s.id);
        let tools = self.tools.iter().map(|t| t.id);
        let toolpaths = self.toolpaths.iter().map(|p| p.id);
        let contours = self.contours.iter().map(|c| c.id);
        setups.chain(tools).chain(toolpaths).chain(contours)
    }

    /// The setup with identity `id`.
    pub fn setup(&self, id: u64) -> Option<&Setup> {
        self.setups.iter().find(|s| s.id == id)
    }

    /// The tool with identity `id`.
    pub fn tool(&self, id: u64) -> Option<&Tool> {
        self.tools.iter().find(|t| t.id == id)
    }

    /// The toolpaths that run in setup `setup`, in the order they were made.
    pub fn toolpaths_in(&self, setup: u64) -> impl Iterator<Item = &Toolpath> + '_ {
        self.toolpaths.iter().filter(move |p| p.setup == setup)
    }

    /// The 2D contour operation with identity `id`.
    pub fn contour(&self, id: u64) -> Option<&CamContour2d> {
        self.contours.iter().find(|c| c.id == id)
    }
}

/// A machining setup: the machine zero a part is cut from, the raw stock it is cut out of, and
/// the bodies it machines.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the browser and in a toolpath's program.
    pub name: String,
    /// The [`CoordinateSystem`] whose machine zero and axes it works from; 0 is the model
    /// origin with the model's own axes.
    pub coordinate_system: u64,
    /// The raw stock the setup cuts from.
    pub stock: Stock,
    /// The bodies it machines, by name; empty means every solid body of the design.
    pub bodies: Vec<String>,
}

/// The raw stock a setup cuts a part out of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stock {
    /// Its size along the setup's X, Y and Z, in mm. A zero size takes the machined bodies'
    /// bounding box along that axis.
    pub size: [f64; 3],
    /// Where its corner nearest the machine zero sits, relative to that zero, in mm.
    pub offset: [f64; 3],
}

impl Default for Stock {
    fn default() -> Self {
        Stock { size: [0.0; 3], offset: [0.0; 3] }
    }
}

/// The kind of cutter a tool library entry describes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// A flat-ended cutter with a square corner: the usual cutter for facing, contouring and
    /// pocketing.
    #[default]
    FlatEndMill,
    /// A ball-nose cutter: a hemisphere on the end, for the curves of a 3D surface.
    BallEndMill,
    /// A bull-nose cutter: a flat end whose corners are rounded by `corner_radius`.
    BullNoseEndMill,
}

/// One cutter of the design's tool library.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tool {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the tool library and in a toolpath's program.
    pub name: String,
    /// What shape its cutting end has.
    pub kind: ToolKind,
    /// The diameter it cuts, in mm.
    pub diameter: f64,
    /// The radius its cutting corners are rounded by, in mm (0 for a square corner).
    pub corner_radius: f64,
    /// The length of its cutting flutes, in mm: the deepest it may cut in one go.
    pub flute_length: f64,
    /// How many cutting edges it has.
    pub flutes: u32,
}

/// The kind of operation a toolpath runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolpathKind {
    /// Flatten the top of the stock, pass by pass across it.
    #[default]
    Facing,
    /// Follow the part's outline with the side of the cutter.
    Contour,
    /// Clear the material inside the part's outline.
    Pocket,
}

/// One machining operation: a kind, a setup and a tool, with the cutting values it runs at.
///
/// The values are the operation's own, in mm and minutes, so changing the tool library never
/// changes a toolpath that was already made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toolpath {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the browser and in its program.
    pub name: String,
    /// What operation it runs.
    pub kind: ToolpathKind,
    /// The [`Setup`] it runs in: its machine zero and its stock.
    pub setup: u64,
    /// The [`Tool`] it cuts with.
    pub tool: u64,
    /// The bodies it cuts, by name; empty means the setup's bodies.
    pub bodies: Vec<String>,
    /// How fast the cutter moves through material, in mm per minute.
    pub feed_rate: f64,
    /// How fast the cutter enters the material, in mm per minute.
    pub plunge_rate: f64,
    /// How fast the spindle turns, in revolutions per minute.
    pub spindle_speed: f64,
    /// How far the cutter moves over between neighbouring passes, in mm.
    pub stepover: f64,
    /// How deep one pass cuts, in mm.
    pub depth_per_pass: f64,
    /// How deep the operation cuts below the top of the stock, in mm.
    pub total_depth: f64,
    /// How much material the operation leaves for a finishing pass, in mm.
    pub stock_to_leave: f64,
}

impl Default for Toolpath {
    fn default() -> Self {
        Toolpath {
            id: 0,
            name: String::new(),
            kind: ToolpathKind::default(),
            setup: 0,
            tool: 0,
            bodies: Vec::new(),
            feed_rate: 0.0,
            plunge_rate: 0.0,
            spindle_speed: 0.0,
            stepover: 0.0,
            depth_per_pass: 0.0,
            total_depth: 0.0,
            stock_to_leave: 0.0,
        }
    }
}

/// A 2D contour operation: a profile toolpath that offsets the edges the user picked by the
/// tool's radius and cuts them, pass by pass, down to a depth.
///
/// Unlike a [`Toolpath`], which contours the whole outline of the bodies it machines, a contour
/// works from named edges of the part, so it can follow a wall or a slot the outline does not
/// bound. The engine reads the exact B-rep to generate the path; nothing here is geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CamContour2d {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the browser and in its program.
    pub name: String,
    /// The [`Setup`] it runs in: its machine zero, axes and stock.
    pub setup: u64,
    /// The [`Tool`] it cuts with.
    pub tool: u64,
    /// The edges it follows, by their persistent names
    /// (`crate::persistent_name::EdgeName`), so they follow the part as it changes.
    pub target_edges: Vec<String>,
    /// The height it travels at between passes, above the top of the stock, in mm.
    pub clearance_height: f64,
    /// How deep the operation cuts below the top of the stock, in mm.
    pub bottom_depth: f64,
    /// How deep one pass cuts, in mm.
    pub stepdown: f64,
}

impl Default for CamContour2d {
    fn default() -> Self {
        CamContour2d {
            id: 0,
            name: String::new(),
            setup: 0,
            tool: 0,
            target_edges: Vec::new(),
            clearance_height: 0.0,
            bottom_depth: 0.0,
            stepdown: 0.0,
        }
    }
}

fn model_x() -> [f64; 3] {
    [1.0, 0.0, 0.0]
}

fn model_y() -> [f64; 3] {
    [0.0, 1.0, 0.0]
}

fn model_z() -> [f64; 3] {
    [0.0, 0.0, 1.0]
}
