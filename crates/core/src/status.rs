use serde::{Deserialize, Serialize};
use zvariant::{Type, as_value::optional as opt};

/// What ryukazed reports in one call, as an `a{sv}` dictionary. A field is
/// missing when the driver or this EC firmware doesn't expose it; clients
/// ignore keys they don't know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[zvariant(signature = "dict")]
#[serde(default)]
pub struct Status {
    /// EC firmware version, e.g. `17KKIMS1.115`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub firmware: Option<String>,

    /// EC firmware build date as the driver prints it.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub firmware_date: Option<String>,

    /// CPU/GPU power preset: `eco`, `comfort`, `sport` or `turbo`, or
    /// `unspecified`/`unknown (N)` for values the driver can't name.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub shift_mode: Option<String>,

    /// The shift modes this firmware offers.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub shift_modes: Option<Vec<String>>,

    /// `auto`, `silent`, `basic` or `advanced`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub fan_mode: Option<String>,

    /// The fan modes this firmware offers.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub fan_modes: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub super_battery: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub cooler_boost: Option<bool>,

    /// Battery charge end threshold in percent.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub charge_limit: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::{LE, serialized::Context, to_bytes};

    #[test]
    fn missing_fields_round_trip() {
        let status = Status {
            shift_mode: Some("turbo".into()),
            charge_limit: Some(80),
            ..Status::default()
        };
        let bytes = to_bytes(Context::new_dbus(LE, 0), &status).unwrap();
        let (decoded, _): (Status, _) = bytes.deserialize().unwrap();
        assert_eq!(decoded, status);
    }
}
