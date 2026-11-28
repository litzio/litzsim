use super::{Circuit, Error, PinKind};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelWarning<'a> {
    pub code: &'static str,
    pub part_id: &'a str,
    pub text: &'a str,
    pub message: String,
}

#[derive(Default)]
pub(crate) struct Net<'a> {
    pub(crate) terminals: Vec<&'a str>,
    pub(crate) signal: bool,
    labels: Vec<(&'a str, &'a str)>,
}

impl Net<'_> {
    pub(crate) fn describe_labels(&self, mut error: Error, use_label_target: bool) -> Error {
        if let Some(&(id, _)) = self.labels.first() {
            let labels = self
                .labels
                .iter()
                .map(|(id, text)| format!("{id} ({text:?})"))
                .collect::<Vec<_>>()
                .join(", ");
            error.message.push_str(&format!("; net labels: {labels}"));
            if use_label_target {
                error.target_id = Some(id.to_owned());
            }
        }
        error
    }
}

pub(crate) struct Connections<'a> {
    pub(crate) nets: BTreeMap<&'a str, Net<'a>>,
    pin_nets: BTreeMap<&'a str, &'a str>,
    drawn_to_resolved: BTreeMap<&'a str, &'a str>,
    pub(crate) warnings: Vec<LabelWarning<'a>>,
}

impl<'a> Connections<'a> {
    pub(crate) fn net_of(&self, terminal: &str) -> Option<&'a str> {
        self.pin_nets.get(terminal).copied()
    }

    pub(crate) fn signal_nets(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.drawn_to_resolved
            .iter()
            .filter_map(|(&drawn, resolved)| self.nets[resolved].signal.then_some(drawn))
    }

    pub(super) fn validate(&self, circuit: &Circuit) -> Result<(), Error> {
        for net in self.nets.values() {
            circuit
                .validate_net(net.terminals.iter().copied())
                .map_err(|error| net.describe_labels(error, true))?;
        }
        Ok(())
    }
}

pub(super) fn read_text<'a>(id: &str, value: Option<&'a Value>) -> Result<&'a str, Error> {
    value
        .and_then(Value::as_str)
        .filter(|text| {
            !text.trim().is_empty()
                && !text
                    .chars()
                    .any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}'))
        })
        .ok_or_else(|| Error::invalid(id, "Label text must be a nonempty single line"))
}

fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

impl Circuit {
    /// Resolve the graph once for validation, drawing metadata and simulation.
    /// Representatives use the first file net ID to preserve solver ordering
    /// for circuits without labels.
    pub(crate) fn connections(&self) -> Connections<'_> {
        let drawn: Vec<_> = self.nets.keys().map(String::as_str).collect();
        let mut parents: Vec<_> = (0..drawn.len()).collect();
        let by_pin: BTreeMap<_, _> = drawn
            .iter()
            .enumerate()
            .flat_map(|(i, &net)| self.nets[net].iter().map(move |pin| (pin.as_str(), i)))
            .collect();
        let mut labels: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for (id, part) in self.parts.iter().filter(|(_, part)| part.kind == "label") {
            let text = part.parameters["text"].as_str().expect("validated label");
            labels.entry(text.trim().to_lowercase()).or_default().push((
                id.as_str(),
                text,
                by_pin.get(format!("{id}.p").as_str()).copied(),
            ));
        }

        let mut warnings = Vec::new();
        for group in labels.values() {
            if let [(id, text, _)] = group.as_slice() {
                warnings.push(LabelWarning {
                    code: "unpairedLabel",
                    part_id: id,
                    text,
                    message: format!("Label {id} ({text:?}) has no matching label"),
                });
            }
            let mut connected = group.iter().filter_map(|&(_, _, net)| net);
            if let Some(first) = connected.next() {
                for net in connected {
                    let a = root(&mut parents, first);
                    let b = root(&mut parents, net);
                    parents[a.max(b)] = a.min(b);
                }
            }
        }

        let mut nets = BTreeMap::<_, Net<'_>>::new();
        let mut drawn_to_resolved = BTreeMap::new();
        let mut pin_nets = BTreeMap::new();
        for (index, &drawn_net) in drawn.iter().enumerate() {
            let representative = drawn[root(&mut parents, index)];
            drawn_to_resolved.insert(drawn_net, representative);
            let net = nets.entry(representative).or_default();
            for terminal in &self.nets[drawn_net] {
                pin_nets.insert(terminal.as_str(), representative);
                let kind = self.require_pin(terminal).expect("validated terminal");
                net.signal |= kind.is_signal();
                if kind != PinKind::NetLabel {
                    net.terminals.push(terminal);
                }
            }
        }
        for (id, text, index) in labels.into_values().flatten() {
            if let Some(index) = index {
                let representative = drawn[root(&mut parents, index)];
                nets.get_mut(representative)
                    .unwrap()
                    .labels
                    .push((id, text));
            }
        }
        Connections {
            nets,
            pin_nets,
            drawn_to_resolved,
            warnings,
        }
    }
}
