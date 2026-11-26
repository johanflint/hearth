use chrono::{DateTime, Utc};
use std::collections::BTreeSet;
use strum::{Display, EnumIter, EnumString};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct User {
    pub id: String,
    pub name: String,
    pub permissions: BTreeSet<Permission>,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn has(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Display, EnumString, EnumIter)]
#[strum(serialize_all = "camelCase")]
pub enum Permission {
    ReadDevices,
    ReadFlows,
    WriteFlows,
    ReadPresets,
    WritePresets,
    ReadLayout, // Groups and zones
    WriteLayout,
    ReadAttention,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::str::FromStr;
    use strum::IntoEnumIterator;

    fn user_with(permissions: BTreeSet<Permission>) -> User {
        User { id: "user-1".to_string(), name: "johan".to_string(), permissions, created_at: Utc::now() }
    }

    #[test]
    fn has_returns_true_for_a_granted_permission() {
        let user = user_with(BTreeSet::from([Permission::ReadDevices, Permission::WriteFlows]));

        assert!(user.has(Permission::ReadDevices));
        assert!(user.has(Permission::WriteFlows));
    }

    #[test]
    fn has_returns_false_for_a_permission_that_is_not_granted() {
        let user = user_with(BTreeSet::from([Permission::ReadFlows]));

        assert!(!user.has(Permission::WriteFlows));
    }

    #[test]
    fn has_returns_false_for_a_user_without_permissions() {
        let user = user_with(BTreeSet::new());

        assert!(Permission::iter().all(|permission| !user.has(permission)));
    }

    // The strings are persisted, so changing one requires a migration
    #[rstest]
    #[case::read_devices(Permission::ReadDevices, "readDevices")]
    #[case::read_flows(Permission::ReadFlows, "readFlows")]
    #[case::write_flows(Permission::WriteFlows, "writeFlows")]
    #[case::read_presets(Permission::ReadPresets, "readPresets")]
    #[case::write_presets(Permission::WritePresets, "writePresets")]
    #[case::read_layout(Permission::ReadLayout, "readLayout")]
    #[case::write_layout(Permission::WriteLayout, "writeLayout")]
    #[case::read_attention(Permission::ReadAttention, "readAttention")]
    fn permission_serializes_to_camel_case(#[case] permission: Permission, #[case] expected: &str) {
        assert_eq!(permission.to_string(), expected);
        assert_eq!(Permission::from_str(expected), Ok(permission));
    }

    #[test]
    fn every_permission_round_trips_through_its_string() {
        for permission in Permission::iter() {
            assert_eq!(Permission::from_str(&permission.to_string()), Ok(permission));
        }
    }

    #[rstest]
    #[case::unknown("deleteEverything")]
    #[case::pascal_case("ReadDevices")]
    #[case::empty("")]
    fn parsing_an_unknown_permission_fails(#[case] value: &str) {
        assert!(Permission::from_str(value).is_err());
    }
}
