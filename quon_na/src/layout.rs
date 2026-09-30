use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x_um: f64,
    pub y_um: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomSite {
    pub id: SiteId,
    pub position: Position,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AodTrapRef {
    pub aod_id: u32,
    pub row: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TrapBinding {
    Slm { site: SiteId },
    Aod { site: SiteId, aod: AodTrapRef },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomBinding {
    pub atom: AtomId,
    pub trap: TrapBinding,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomLayout {
    pub sites: Vec<AtomSite>,
    /// Working occupancy. Zoned scheduling and flat AOD movement rewrite this
    /// to the **final** sites. The emitted-schedule replay must not start here.
    pub initial_bindings: Vec<AtomBinding>,
    /// Bindings at placement time. Recorded once, before those rewrites, and
    /// kept when a later phase updates [`Self::initial_bindings`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_initial_bindings: Vec<AtomBinding>,
}

impl NeutralAtomLayout {
    /// Remember the current bindings as the replay start, if that has not
    /// already been recorded. Later rewrites of [`Self::initial_bindings`]
    /// leave this vector alone.
    pub fn preserve_declared_start(&mut self) {
        if self.declared_initial_bindings.is_empty() && !self.initial_bindings.is_empty() {
            self.declared_initial_bindings = self.initial_bindings.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn layout_round_trips_through_json() {
        let layout = NeutralAtomLayout {
            sites: vec![AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 1.25,
                    y_um: 2.5,
                },
            }],
            initial_bindings: vec![AtomBinding {
                atom: AtomId(7),
                trap: TrapBinding::Aod {
                    site: SiteId(0),
                    aod: AodTrapRef {
                        aod_id: 3,
                        row: 4,
                        col: 5,
                    },
                },
            }],

            declared_initial_bindings: Vec::new(),
        };

        let value = match serde_json::to_value(&layout) {
            Ok(value) => value,
            Err(error) => panic!("neutral atom layout serialization failed: {error}"),
        };
        let decoded: NeutralAtomLayout = match serde_json::from_value(value) {
            Ok(decoded) => decoded,
            Err(error) => panic!("neutral atom layout deserialization failed: {error}"),
        };

        assert_eq!(decoded, layout);
    }

    #[test]
    fn layout_rejects_unknown_json_fields() {
        let value = json!({
            "sites": [],
            "initial_bindings": [],
            "extra": "field",
        });

        assert!(serde_json::from_value::<NeutralAtomLayout>(value).is_err());
    }
}
