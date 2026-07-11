//! The firmware images bundled into the app, so that creating a save never
//! requires the player to find image files. These are the same dumps the web
//! frontend ships (`web/fw/`); 1.09.03 exists for every character and is the
//! version players are guided to. The later 2.x dumps are reachable only
//! through the advanced options, alongside fully custom images.

/// The OTP (boot ROM) image shared by every retail handheld.
pub static OTP: &[u8] = include_bytes!("../../web/fw/OTP.dat");

/// One bundled flash image: a character at a firmware version.
pub struct BundledFirmware {
    pub character: &'static str,
    pub version: &'static str,
    pub data: &'static [u8],
}

/// Every bundled image, grouped character-major. 1.09.03 first per character.
pub static FIRMWARE: &[BundledFirmware] = &[
    BundledFirmware {
        character: "Cloe",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Cloe 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Cloe",
        version: "2.03.01",
        data: include_bytes!("../../web/fw/Cloe 2.03.01.dat"),
    },
    BundledFirmware {
        character: "Creeper",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Creeper 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Dash",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Dash 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Inferno",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Inferno 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Inferno",
        version: "2.00.04",
        data: include_bytes!("../../web/fw/Inferno 2.00.04.dat"),
    },
    BundledFirmware {
        character: "Roc",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Roc 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Roc",
        version: "2.00.04",
        data: include_bytes!("../../web/fw/Roc 2.00.04.dat"),
    },
    BundledFirmware {
        character: "Spike",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Spike 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Yasmin",
        version: "1.09.03",
        data: include_bytes!("../../web/fw/Yasmin 1.09.03.dat"),
    },
    BundledFirmware {
        character: "Yasmin",
        version: "2.03.01",
        data: include_bytes!("../../web/fw/Yasmin 2.03.01.dat"),
    },
];

/// The firmware version new saves are guided to.
pub const RECOMMENDED_VERSION: &str = "1.09.03";

/// The characters offered by the new-save flow, in display order.
/// Monsterz first, then the Bratz pair, matching how the line was sold.
pub const CHARACTERS: &[&str] = &[
    "Spike", "Inferno", "Creeper", "Dash", "Roc", "Cloe", "Yasmin",
];

pub fn find(character: &str, version: &str) -> Option<&'static BundledFirmware> {
    FIRMWARE
        .iter()
        .find(|fw| fw.character == character && fw.version == version)
}

/// The versions bundled for a character, recommended version first.
pub fn versions_for(character: &str) -> Vec<&'static str> {
    let mut versions: Vec<&'static str> = FIRMWARE
        .iter()
        .filter(|fw| fw.character == character)
        .map(|fw| fw.version)
        .collect();
    versions.sort_by_key(|v| (*v != RECOMMENDED_VERSION, *v));
    versions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_character_has_the_recommended_firmware() {
        for character in CHARACTERS {
            assert!(
                find(character, RECOMMENDED_VERSION).is_some(),
                "{character} is missing {RECOMMENDED_VERSION}"
            );
        }
    }

    #[test]
    fn bundled_images_have_flash_size() {
        for fw in FIRMWARE {
            assert_eq!(fw.data.len(), 2 * 1024 * 1024, "{}", fw.character);
        }
    }

    #[test]
    fn versions_list_recommended_first() {
        assert_eq!(versions_for("Cloe")[0], RECOMMENDED_VERSION);
        assert_eq!(versions_for("Roc")[0], RECOMMENDED_VERSION);
    }
}
