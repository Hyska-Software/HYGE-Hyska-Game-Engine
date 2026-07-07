//! Structured audio bus mixer math.

/// Named engine audio buses.
#[derive(bevy_reflect::Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BusKind {
    /// Sound effects.
    #[default]
    Sfx,
    /// Spoken dialog / voice-over.
    Voice,
    /// User-interface sounds.
    Ui,
    /// Ambient loops and diegetic beds.
    Ambient,
    /// Music bus.
    Music,
}

/// Linear volume for each bus and the master bus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioBusVolumes {
    /// Master output gain applied to every bus.
    pub master: f32,
    /// SFX gain.
    pub sfx: f32,
    /// Voice gain.
    pub voice: f32,
    /// UI gain.
    pub ui: f32,
    /// Ambient gain.
    pub ambient: f32,
    /// Music gain.
    pub music: f32,
}

impl Default for AudioBusVolumes {
    fn default() -> Self {
        Self {
            master: 1.0,
            sfx: 1.0,
            voice: 1.0,
            ui: 1.0,
            ambient: 1.0,
            music: 1.0,
        }
    }
}

impl AudioBusVolumes {
    /// Returns a bus's local gain, without master gain.
    #[must_use]
    pub fn local_gain(&self, bus: BusKind) -> f32 {
        match bus {
            BusKind::Sfx => self.sfx,
            BusKind::Voice => self.voice,
            BusKind::Ui => self.ui,
            BusKind::Ambient => self.ambient,
            BusKind::Music => self.music,
        }
    }

    /// Returns the final inherited gain for a source routed to `bus`.
    #[must_use]
    pub fn inherited_gain(&self, bus: BusKind) -> f32 {
        self.master * self.local_gain(bus)
    }
}

/// Runtime bus mixer state.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuses {
    /// Current volume state for all buses.
    pub volumes: AudioBusVolumes,
    /// Master bus label.
    pub master: String,
    /// SFX bus label.
    pub sfx: String,
    /// Voice bus label.
    pub voice: String,
    /// UI bus label.
    pub ui: String,
    /// Ambient bus label.
    pub ambient: String,
    /// Music bus label.
    pub music: String,
}

impl Default for AudioBuses {
    fn default() -> Self {
        Self {
            volumes: AudioBusVolumes::default(),
            master: "master".to_string(),
            sfx: "sfx".to_string(),
            voice: "voice".to_string(),
            ui: "ui".to_string(),
            ambient: "ambient".to_string(),
            music: "music".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_volume_inherits_master_gain() {
        let volumes = AudioBusVolumes {
            master: 0.5,
            sfx: 0.25,
            ..AudioBusVolumes::default()
        };

        assert_eq!(volumes.inherited_gain(BusKind::Sfx), 0.125);
    }

    #[test]
    fn all_buses_have_expected_default_gain() {
        let volumes = AudioBusVolumes::default();
        for bus in [
            BusKind::Sfx,
            BusKind::Voice,
            BusKind::Ui,
            BusKind::Ambient,
            BusKind::Music,
        ] {
            assert_eq!(volumes.inherited_gain(bus), 1.0);
        }
    }
}
