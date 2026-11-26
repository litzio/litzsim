use super::{
    GRID_PITCH, Issue, JUMP_RADIUS, Jump, Junction, Layout, Obstacle, Pin, Point, Rect, Segment,
    Target, add, scene,
};
use crate::circuit::Document;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn between(a: Point, b: Point) -> Self {
        if a[1] == b[1] {
            Self::Horizontal
        } else {
            Self::Vertical
        }
    }
}

type Tree = BTreeMap<Point, Vec<Point>>;
type GridPins<'a> = BTreeMap<Point, Vec<(&'a str, &'a Pin<'a>)>>;
type State = (Point, Option<Axis>);
type Cost = (usize, i64);

// Search a grid of representative rows and columns rather than stepping over
// every empty cell between distant terminals. The steps still follow the grid.
#[derive(Default)]
struct Grid {
    x: Vec<i64>,
    y: Vec<i64>,
}

impl Grid {
    fn new(points: impl Iterator<Item = Point>, boxes: &[Obstacle<'_>]) -> Self {
        let mut grid = Self::default();
        for [x, y] in points {
            grid.x.extend([x - GRID_PITCH, x, x + GRID_PITCH]);
            grid.y.extend([y - GRID_PITCH, y, y + GRID_PITCH]);
        }
        for obstacle in boxes {
            let Rect { min, max } = obstacle.bounds;
            let before = |n: i64| (n - 1).div_euclid(GRID_PITCH) * GRID_PITCH;
            let after = |n: i64| (n.div_euclid(GRID_PITCH) + 1) * GRID_PITCH;
            grid.x.extend([before(min[0]), after(max[0])]);
            grid.y.extend([before(min[1]), after(max[1])]);
        }
        grid.x.sort_unstable();
        grid.x.dedup();
        grid.y.sort_unstable();
        grid.y.dedup();
        grid
    }

    fn neighbors(&self, [x, y]: Point) -> impl Iterator<Item = Point> {
        let column = self.x.binary_search(&x).expect("routing column");
        let row = self.y.binary_search(&y).expect("routing row");
        [
            self.x.get(column + 1).map(|&next| [next, y]),
            self.y.get(row + 1).map(|&next| [x, next]),
            column.checked_sub(1).map(|i| [self.x[i], y]),
            row.checked_sub(1).map(|i| [x, self.y[i]]),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Clone, Copy)]
struct Occupied<'a> {
    net: &'a str,
    // Only a clear straight run may be crossed by a later path.
    crossable: Option<Axis>,
}

struct Board<'a, 'b> {
    grid: Grid,
    obstacles: &'b [Obstacle<'a>],
    pins: GridPins<'b>,
    bends: BTreeMap<Point, BTreeSet<&'a str>>,
    wires: BTreeMap<Point, Occupied<'a>>,
}

fn distance(a: Point, b: Point) -> i64 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs()
}

// Exact cost of the best unobstructed path, with the incoming axis preserved.
fn estimate((point, previous_axis): State, goal: Point) -> Cost {
    let turns = match (point[0] != goal[0], point[1] != goal[1]) {
        (true, true) => 1,
        (true, false) => usize::from(previous_axis == Some(Axis::Vertical)),
        (false, true) => usize::from(previous_axis == Some(Axis::Horizontal)),
        (false, false) => 0,
    };
    (turns, distance(point, goal))
}

impl Board<'_, '_> {
    fn path(
        &self,
        net: &str,
        start: Point,
        incoming_axis: Option<Axis>,
        goals: &BTreeSet<Point>,
        avoid: &BTreeSet<Point>,
    ) -> Option<Vec<Point>> {
        if goals.is_empty()
            || self.wires.contains_key(&start)
            || !self.allows_node(net, start, true)
        {
            return None;
        }
        let lower_bound = |state| {
            goals
                .iter()
                .map(|&goal| estimate(state, goal))
                .min()
                .unwrap()
        };
        let initial = (start, incoming_axis);
        let start_cost: Cost = (0, 0);
        let mut best = HashMap::from([(initial, (start_cost, None))]);
        let mut queue = BinaryHeap::new();
        let mut sequence = 0;
        let initial_bound = lower_bound(initial);
        queue.push(Reverse((
            initial_bound,
            initial_bound,
            sequence,
            start_cost,
            initial,
        )));

        // Keep a heavily obstructed edit from searching the full Cartesian grid.
        const MAX_EXPANDED: usize = 4096;
        let mut expanded = 0;
        while let Some(Reverse((_, _, _, cost, state @ (point, previous_axis)))) = queue.pop() {
            if best[&state].0 != cost {
                continue;
            }
            if goals.contains(&point) && !self.wires.contains_key(&point) {
                let mut path = vec![point];
                let mut current = state;
                while let Some(parent) = best[&current].1 {
                    current = parent;
                    path.push(current.0);
                }
                path.reverse();
                return Some(path);
            }
            expanded += 1;
            if expanded > MAX_EXPANDED {
                break;
            }
            for next in self.grid.neighbors(point) {
                let goal = goals.contains(&next);
                let axis = Axis::between(point, next);
                if next == start
                    || (avoid.contains(&next) && !goal)
                    || !self.allows_node(net, next, goal)
                    || !self.allows_edge(point, next, axis)
                {
                    continue;
                }
                let next_state = (next, Some(axis));
                let turn = usize::from(previous_axis.is_some_and(|prev| prev != axis));
                let next_cost = (cost.0 + turn, cost.1 + distance(point, next));
                if best
                    .get(&next_state)
                    .is_some_and(|(old, _)| *old <= next_cost)
                {
                    continue;
                }
                best.insert(next_state, (next_cost, Some(state)));
                let bound = lower_bound(next_state);
                sequence += 1;
                queue.push(Reverse((
                    (next_cost.0 + bound.0, next_cost.1 + bound.1),
                    bound,
                    sequence,
                    next_cost,
                    next_state,
                )));
            }
        }
        None
    }

    fn allows_node(&self, net: &str, point: Point, endpoint: bool) -> bool {
        if let Some(pins) = self.pins.get(&point)
            && (!endpoint || pins.iter().any(|(_, pin)| pin.net_id != Some(net)))
        {
            return false;
        }
        self.bends
            .get(&point)
            .is_none_or(|nets| nets.len() == 1 && nets.contains(net))
    }

    fn allows_edge(&self, a: Point, b: Point, axis: Axis) -> bool {
        for point in [a, b] {
            if let Some(wire) = self.wires.get(&point)
                && !wire.crossable.is_some_and(|other| other != axis)
            {
                return false;
            }
        }
        let wire = Rect::between(a, b);
        self.obstacles.iter().all(|obstacle| {
            let Some(contact) = wire.intersection(obstacle.bounds) else {
                return true;
            };
            obstacle.symbol.is_some_and(|(part, center)| {
                let owns_pin = |at| {
                    self.pins
                        .get(&at)
                        .is_some_and(|pins| pins.iter().any(|(_, pin)| pin.part_id == part))
                };
                if contact.min == contact.max
                    && (contact.min == a || contact.min == b)
                    && owns_pin(contact.min)
                {
                    return true;
                }
                // The terminal face contains only the tip of its lead. Meeting
                // it tangentially lets opposing tips share a straight rail.
                let normal = if axis == Axis::Horizontal { 1 } else { 0 };
                let mut tip = center;
                tip[normal] = a[normal];
                (a[normal] == obstacle.bounds.min[normal]
                    || a[normal] == obstacle.bounds.max[normal])
                    && owns_pin(tip)
            })
        })
    }

    fn jump_fits(&self, point: Point) -> bool {
        let rim = JUMP_RADIUS + 4;
        let envelope = Rect::between(add(point, [-rim, -rim]), add(point, [rim, 4]));
        self.obstacles
            .iter()
            .all(|obstacle| envelope.intersection(obstacle.bounds).is_none())
    }

    fn branch(
        &self,
        net: &str,
        start: Point,
        waypoints: &[Point],
        goals: &BTreeSet<Point>,
        tree: &Tree,
    ) -> Option<Vec<Point>> {
        if tree.contains_key(&start) {
            // Adding a path from a coincident terminal back into the tree
            // would create a cycle and make a wire's terminal sides ambiguous.
            return (goals.contains(&start)
                && waypoints.iter().all(|&point| point == start)
                && self.allows_node(net, start, true))
            .then_some(vec![start]);
        }
        let mut path = vec![start];
        let mut avoid: BTreeSet<_> = tree.keys().copied().collect();
        let mut incoming_axis = None;
        for (i, &waypoint) in waypoints.iter().enumerate() {
            let previous = *path.last().unwrap();
            if waypoint == previous {
                continue;
            }
            let joins_tree = i + 1 == waypoints.len() && goals.contains(&waypoint);
            if path.contains(&waypoint) || (tree.contains_key(&waypoint) && !joins_tree) {
                return None;
            }
            let leg = self.path(
                net,
                previous,
                incoming_axis,
                &BTreeSet::from([waypoint]),
                &avoid,
            )?;
            incoming_axis = leg.last_chunk().map(|&[a, b]| Axis::between(a, b));
            avoid.extend(leg.iter().take(leg.len() - 1).copied());
            path.extend(leg.into_iter().skip(1));
        }
        let goals = goals
            .iter()
            .filter(|goal| !path[..path.len() - 1].contains(goal))
            .copied()
            .collect();
        let leg = self.path(net, *path.last().unwrap(), incoming_axis, &goals, &avoid)?;
        path.extend(leg.into_iter().skip(1));
        Some(path)
    }
}

pub(super) fn route<'a>(
    document: &'a Document,
    obstacles: &[Obstacle<'a>],
    layout: &mut Layout<'a>,
) {
    let Layout {
        pins,
        segments,
        junctions,
        jumps,
        issues,
        ..
    } = layout;
    let mut grid_pins = GridPins::new();
    for (id, pin) in pins.iter() {
        grid_pins.entry(pin.at).or_default().push((id, pin));
    }
    for (&point, at) in &grid_pins {
        for (i, (a, first)) in at.iter().enumerate() {
            for (b, second) in &at[i + 1..] {
                if first.net_id.is_none() || first.net_id != second.net_id {
                    issues.push(Issue {
                        code: "overlap",
                        targets: [Target::new("pin", a), Target::new("pin", b)],
                        area: Rect::point(point),
                    });
                }
            }
        }
    }

    let mut bends: BTreeMap<Point, BTreeSet<&str>> = BTreeMap::new();
    for (net, terminals) in document.layout_nets() {
        for terminal in terminals.iter().filter(|_| terminals.len() > 1) {
            if let Some(bend) = document.layout_bend(terminal) {
                bends.entry(scene(bend)).or_default().insert(net);
            }
        }
    }
    let mut board = Board {
        grid: Grid::new(grid_pins.keys().chain(bends.keys()).copied(), obstacles),
        obstacles,
        pins: grid_pins,
        bends,
        wires: BTreeMap::new(),
    };

    for (net, ids) in document.layout_nets() {
        if ids.len() < 2 {
            continue;
        }
        // Grow the tree in sheet order, attaching unbent terminals first.
        let mut ids: Vec<_> = ids.iter().collect();
        ids.sort_unstable_by_key(|&id| {
            let [x, y] = pins[id].at;
            (document.layout_bend(id).is_some(), y, x, id)
        });
        let root_id = ids[0];
        let root = pins[root_id].at;
        let root_bend = document.layout_bend(root_id).map(scene);
        let mut tree = Tree::from([(root, Vec::new())]);
        let mut attached = BTreeMap::from([(root, vec![root_id.as_str()])]);
        let mut waypoints = BTreeSet::new();
        let mut connected = false;
        for id in ids.into_iter().skip(1) {
            let start = pins[id].at;
            let goals: BTreeSet<_> = tree
                .keys()
                .filter(|point| !board.wires.contains_key(*point))
                .copied()
                .collect();
            let required: Vec<_> = document
                .layout_bend(id)
                .map(scene)
                .into_iter()
                .chain(root_bend.filter(|_| !connected))
                .collect();
            if let Some(path) = board.branch(net, start, &required, &goals, &tree) {
                for edge in path.windows(2) {
                    let (a, b) = (edge[0], edge[1]);
                    tree.entry(a).or_default().push(b);
                    tree.entry(b).or_default().push(a);
                }
                attached.entry(start).or_default().push(id);
                waypoints.extend(required);
                connected = true;
            } else {
                let area = required
                    .iter()
                    .fold(Rect::between(start, root), |area, &point| {
                        area.union(Rect::point(point))
                    });
                issues.push(Issue {
                    code: "routeFailed",
                    targets: [Target::new("pin", id), Target::new("pin", root_id)],
                    area,
                });
            }
        }

        for (&point, neighbors) in &tree {
            let terminal_count = attached.get(&point).map_or(0, Vec::len);
            if neighbors.len() + terminal_count >= 3 {
                junctions.push(Junction {
                    at: point,
                    net_id: net,
                });
            }
            let straight = straight_axis(point, neighbors);
            if let Some(other) = board.wires.get(&point) {
                if let (Some(axis), Some(other_axis)) = (straight, other.crossable) {
                    debug_assert_ne!(axis, other_axis);
                    let (horizontal, vertical) = if axis == Axis::Horizontal {
                        (net.as_str(), other.net)
                    } else {
                        (other.net, net.as_str())
                    };
                    jumps.push(Jump {
                        at: point,
                        radius: JUMP_RADIUS,
                        horizontal_net_id: horizontal,
                        vertical_net_id: vertical,
                    });
                    // A third net may not visit this point.
                    board.wires.insert(
                        point,
                        Occupied {
                            net,
                            crossable: None,
                        },
                    );
                }
            } else if !neighbors.is_empty() {
                let crossable = straight.filter(|_| {
                    terminal_count == 0
                        && !board.bends.contains_key(&point)
                        && board.jump_fits(point)
                });
                board.wires.insert(point, Occupied { net, crossable });
            }
        }
        collect_segments(&tree, &attached, &waypoints, root, root_id, net, segments);
    }
}

fn straight_axis(point: Point, neighbors: &[Point]) -> Option<Axis> {
    if let &[a, b] = neighbors {
        let axis = Axis::between(point, a);
        return (axis == Axis::between(point, b)).then_some(axis);
    }
    None
}

fn collect_segments<'a>(
    tree: &Tree,
    pins: &BTreeMap<Point, Vec<&'a str>>,
    waypoints: &BTreeSet<Point>,
    root: Point,
    root_id: &'a str,
    net: &'a str,
    segments: &mut Vec<Segment<'a>>,
) {
    // The lexicographically first terminal in each subtree represents its side
    // of the cut. The root terminal represents the other side.
    let mut order = Vec::new();
    let mut todo = vec![(root, root)];
    while let Some((point, parent)) = todo.pop() {
        order.push((point, parent));
        todo.extend(
            tree[&point]
                .iter()
                .filter(|&&next| next != parent)
                .map(|&next| (next, point)),
        );
    }
    let mut representative: BTreeMap<_, _> = pins
        .iter()
        .map(|(&at, ids)| (at, *ids.iter().min().unwrap()))
        .collect();
    for &(point, parent) in order.iter().rev().take(order.len() - 1) {
        let terminal = representative[&point];
        let previous = representative.entry(parent).or_insert(terminal);
        *previous = (*previous).min(terminal);
    }

    for &(point, parent) in order.iter().skip(1) {
        let keep = |at| {
            pins.contains_key(&at)
                || waypoints.contains(&at)
                || straight_axis(at, &tree[&at]).is_none()
        };
        if keep(point) {
            let (mut at, mut child) = (parent, point);
            while !keep(at) {
                let next = *tree[&at].iter().find(|&&next| next != child).unwrap();
                (at, child) = (next, at);
            }
            segments.push(Segment {
                from: at,
                to: point,
                net_id: net,
                terminals: [root_id, representative[&point]],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_cost_prefers_fewer_turns_then_shorter_length() {
        let start = [0, 0];
        let goals = BTreeSet::from([[32, 32], [128, 0], [160, 0]]);
        let board = Board {
            grid: Grid::new(goals.iter().copied().chain([start]), &[]),
            obstacles: &[],
            pins: BTreeMap::new(),
            bends: BTreeMap::new(),
            wires: BTreeMap::new(),
        };
        let path = board
            .path("net", start, None, &goals, &BTreeSet::new())
            .unwrap();
        assert_eq!(path.last(), Some(&[128, 0]));
        assert!(path.iter().all(|point| point[1] == 0));
    }
}
