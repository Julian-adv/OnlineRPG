use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalProtection {
    pub slash: u32,
    pub pierce: u32,
    pub blunt: u32,
}

impl PhysicalProtection {
    pub fn for_damage_type(self, damage_type: PhysicalDamageType) -> u32 {
        match damage_type {
            PhysicalDamageType::Untyped => 0,
            PhysicalDamageType::Slash => self.slash,
            PhysicalDamageType::Pierce => self.pierce,
            PhysicalDamageType::Blunt => self.blunt,
        }
    }

    pub fn is_empty(self) -> bool {
        self.slash == 0 && self.pierce == 0 && self.blunt == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalDamageResult {
    pub damage_type: PhysicalDamageType,
    pub raw_damage: u32,
    pub mitigated_damage: u32,
    pub final_damage: u32,
}

pub fn apply_body_coverage(
    protection: PhysicalProtection,
    coverage_percent: u32,
) -> PhysicalProtection {
    let coverage = u64::from(coverage_percent.min(crate::inventory::BODY_COVERAGE_SCALE));
    let scale = |value: u32| {
        if value == 0 || coverage == 0 {
            return 0;
        }
        let scaled = (u64::from(value) * coverage)
            .div_ceil(u64::from(crate::inventory::BODY_COVERAGE_SCALE));
        scaled.min(u64::from(u32::MAX)) as u32
    };
    PhysicalProtection {
        slash: scale(protection.slash),
        pierce: scale(protection.pierce),
        blunt: scale(protection.blunt),
    }
}

pub fn resolve_physical_damage(
    raw_damage: u32,
    damage_type: PhysicalDamageType,
    protection: PhysicalProtection,
) -> PhysicalDamageResult {
    let mitigated_damage = protection
        .for_damage_type(damage_type)
        .min(raw_damage.saturating_sub(1));
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
    fn authored_protection_selects_only_the_matching_physical_channel() {
        let protection = PhysicalProtection {
            slash: 1,
            pierce: 2,
            blunt: 3,
        };
        assert_eq!(protection.for_damage_type(PhysicalDamageType::Untyped), 0);
        assert_eq!(protection.for_damage_type(PhysicalDamageType::Slash), 1);
        assert_eq!(protection.for_damage_type(PhysicalDamageType::Pierce), 2);
        assert_eq!(protection.for_damage_type(PhysicalDamageType::Blunt), 3);
        assert!(!protection.is_empty());
        assert!(PhysicalProtection::default().is_empty());
    }

    #[test]
    fn body_coverage_scales_authored_protection_without_region_rolls() {
        let plate = PhysicalProtection {
            slash: 3,
            pierce: 3,
            blunt: 1,
        };
        assert_eq!(
            apply_body_coverage(plate, 40),
            PhysicalProtection {
                slash: 2,
                pierce: 2,
                blunt: 1,
            }
        );
        assert_eq!(apply_body_coverage(plate, 85), plate);
        assert_eq!(
            apply_body_coverage(
                PhysicalProtection {
                    slash: 2,
                    pierce: 1,
                    blunt: 0,
                },
                75,
            ),
            PhysicalProtection {
                slash: 2,
                pierce: 1,
                blunt: 0,
            }
        );
        assert_eq!(apply_body_coverage(plate, 0), PhysicalProtection::default());
        assert_eq!(apply_body_coverage(plate, 150), plate);

        let mut previous = PhysicalProtection::default();
        for coverage in 0..=100 {
            let current = apply_body_coverage(plate, coverage);
            assert!(current.slash >= previous.slash && current.slash <= plate.slash);
            assert!(current.pierce >= previous.pierce && current.pierce <= plate.pierce);
            assert!(current.blunt >= previous.blunt && current.blunt <= plate.blunt);
            previous = current;
        }
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
                let unarmored =
                    resolve_physical_damage(raw_damage, damage_type, PhysicalProtection::default());
                assert_eq!(unarmored.raw_damage, raw_damage);
                assert_eq!(unarmored.final_damage, raw_damage);

                for protection in [
                    PhysicalProtection {
                        slash: 1,
                        pierce: 0,
                        blunt: 2,
                    },
                    PhysicalProtection {
                        slash: 1,
                        pierce: 1,
                        blunt: 1,
                    },
                    PhysicalProtection {
                        slash: 2,
                        pierce: 1,
                        blunt: 0,
                    },
                    PhysicalProtection {
                        slash: 3,
                        pierce: 3,
                        blunt: 1,
                    },
                    PhysicalProtection {
                        slash: 2,
                        pierce: 2,
                        blunt: 2,
                    },
                ] {
                    let armored = resolve_physical_damage(raw_damage, damage_type, protection);
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
