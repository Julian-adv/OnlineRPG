use serde::{Deserialize, Serialize};

use crate::inventory::ArmorConstruction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalDamageType {
    Untyped,
    Slash,
    Pierce,
    Blunt,
}

impl PhysicalDamageType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Untyped => "untyped",
            Self::Slash => "slash",
            Self::Pierce => "pierce",
            Self::Blunt => "blunt",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Untyped => "Untyped",
            Self::Slash => "Slash",
            Self::Pierce => "Pierce",
            Self::Blunt => "Blunt",
        }
    }
}

impl std::str::FromStr for PhysicalDamageType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "untyped" => Ok(Self::Untyped),
            "slash" => Ok(Self::Slash),
            "pierce" => Ok(Self::Pierce),
            "blunt" => Ok(Self::Blunt),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalDamageResult {
    pub damage_type: PhysicalDamageType,
    pub raw_damage: u32,
    pub mitigated_damage: u32,
    pub final_damage: u32,
}

pub fn construction_protection(
    construction: Option<ArmorConstruction>,
    damage_type: PhysicalDamageType,
) -> u32 {
    match (construction, damage_type) {
        (Some(ArmorConstruction::Padded), PhysicalDamageType::Slash) => 1,
        (Some(ArmorConstruction::Padded), PhysicalDamageType::Blunt) => 2,
        (
            Some(ArmorConstruction::Leather),
            PhysicalDamageType::Slash | PhysicalDamageType::Pierce | PhysicalDamageType::Blunt,
        ) => 1,
        (Some(ArmorConstruction::Mail), PhysicalDamageType::Slash) => 2,
        (Some(ArmorConstruction::Mail), PhysicalDamageType::Pierce) => 1,
        (
            Some(ArmorConstruction::Plate),
            PhysicalDamageType::Slash | PhysicalDamageType::Pierce,
        ) => 3,
        (Some(ArmorConstruction::Plate), PhysicalDamageType::Blunt) => 1,
        (
            Some(ArmorConstruction::Hybrid),
            PhysicalDamageType::Slash | PhysicalDamageType::Pierce | PhysicalDamageType::Blunt,
        ) => 2,
        _ => 0,
    }
}

pub fn resolve_physical_damage(
    raw_damage: u32,
    damage_type: PhysicalDamageType,
    construction: Option<ArmorConstruction>,
) -> PhysicalDamageResult {
    let protection = construction_protection(construction, damage_type);
    let mitigated_damage = protection.min(raw_damage.saturating_sub(1));
    PhysicalDamageResult {
        damage_type,
        raw_damage,
        mitigated_damage,
        final_damage: raw_damage.saturating_sub(mitigated_damage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_types_have_stable_wire_names() {
        for (damage_type, wire, display) in [
            (PhysicalDamageType::Untyped, "untyped", "Untyped"),
            (PhysicalDamageType::Slash, "slash", "Slash"),
            (PhysicalDamageType::Pierce, "pierce", "Pierce"),
            (PhysicalDamageType::Blunt, "blunt", "Blunt"),
        ] {
            assert_eq!(damage_type.as_str(), wire);
            assert_eq!(damage_type.display_name(), display);
            assert_eq!(
                serde_json::to_string(&damage_type).unwrap(),
                format!("\"{wire}\"")
            );
            assert_eq!(
                serde_json::from_str::<PhysicalDamageType>(&format!("\"{wire}\"")).unwrap(),
                damage_type
            );
            assert_eq!(wire.parse::<PhysicalDamageType>(), Ok(damage_type));
        }
        assert!("fire".parse::<PhysicalDamageType>().is_err());
    }

    #[test]
    fn active_construction_profiles_are_explicit() {
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Padded), PhysicalDamageType::Slash),
            1
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Padded), PhysicalDamageType::Pierce),
            0
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Padded), PhysicalDamageType::Blunt),
            2
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Padded), PhysicalDamageType::Untyped),
            0
        );

        for damage_type in [
            PhysicalDamageType::Slash,
            PhysicalDamageType::Pierce,
            PhysicalDamageType::Blunt,
        ] {
            assert_eq!(
                construction_protection(Some(ArmorConstruction::Leather), damage_type),
                1,
                "leather {damage_type:?}"
            );
        }
        assert_eq!(
            construction_protection(
                Some(ArmorConstruction::Leather),
                PhysicalDamageType::Untyped
            ),
            0
        );

        assert_eq!(
            construction_protection(Some(ArmorConstruction::Mail), PhysicalDamageType::Slash),
            2
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Mail), PhysicalDamageType::Pierce),
            1
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Mail), PhysicalDamageType::Blunt),
            0
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Mail), PhysicalDamageType::Untyped),
            0
        );

        for damage_type in [PhysicalDamageType::Slash, PhysicalDamageType::Pierce] {
            assert_eq!(
                construction_protection(Some(ArmorConstruction::Plate), damage_type),
                3,
                "plate {damage_type:?}"
            );
        }
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Plate), PhysicalDamageType::Blunt),
            1
        );
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Plate), PhysicalDamageType::Untyped),
            0
        );

        for damage_type in [
            PhysicalDamageType::Slash,
            PhysicalDamageType::Pierce,
            PhysicalDamageType::Blunt,
        ] {
            assert_eq!(
                construction_protection(Some(ArmorConstruction::Hybrid), damage_type),
                2,
                "hybrid {damage_type:?}"
            );
        }
        assert_eq!(
            construction_protection(Some(ArmorConstruction::Hybrid), PhysicalDamageType::Untyped),
            0
        );
    }

    #[test]
    fn mitigation_is_bounded_monotonic_and_never_zeros_a_hit() {
        for raw_damage in 0..=100 {
            for damage_type in [
                PhysicalDamageType::Untyped,
                PhysicalDamageType::Slash,
                PhysicalDamageType::Pierce,
                PhysicalDamageType::Blunt,
            ] {
                let unarmored = resolve_physical_damage(raw_damage, damage_type, None);
                assert_eq!(unarmored.raw_damage, raw_damage);
                assert_eq!(unarmored.final_damage, raw_damage);

                for construction in [
                    ArmorConstruction::Padded,
                    ArmorConstruction::Leather,
                    ArmorConstruction::Mail,
                    ArmorConstruction::Plate,
                    ArmorConstruction::Hybrid,
                ] {
                    let armored =
                        resolve_physical_damage(raw_damage, damage_type, Some(construction));
                    assert!(armored.final_damage <= raw_damage);
                    assert!(armored.mitigated_damage <= raw_damage);
                    assert_eq!(armored.final_damage + armored.mitigated_damage, raw_damage);
                    if raw_damage > 0 {
                        assert!(armored.final_damage >= 1);
                    }
                }
            }
        }
    }
}
