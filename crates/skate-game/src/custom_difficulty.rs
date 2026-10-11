//! User-owned difficulty values. Original collection hashes remain asset identities;
//! player-facing names describe verified consumers, not guessed original names.
use std::{collections::BTreeMap, path::Path};
use serde::{Deserialize, Serialize};
use skate_data::collections::{Collections, Field};

pub(crate) struct OptionSpec {
    pub key: &'static str, pub label: &'static str, pub description: &'static str,
    pub max: f32, pub step: f32, pub boolean: bool,
}
macro_rules! number {
    ($key:literal,$label:literal,$description:literal,$max:literal,$step:literal) => {
        OptionSpec {key:$key,label:$label,description:$description,max:$max,step:$step,boolean:false}
    }
}
macro_rules! toggle {
    ($key:literal,$label:literal,$description:literal) => {
        OptionSpec {key:$key,label:$label,description:$description,max:1.,step:1.,boolean:true}
    }
}
pub(crate) const OPTIONS: &[OptionSpec] = &[
    toggle!("MotorEnabled","Motor assistance","Hold RB on suitable ground to drive toward the motor target speed."),
    number!("MotorTopSpeed","Motor target speed (km/h)","Target speed while motor assistance is enabled and RB is held.",2000.,1.),
    toggle!("AutoPushEnabled","Automatic pushing","Enable the native automatic-push behavior."),
    number!("HostPushStrength","Push strength (x)","Multiplies the speed gain requested by each push animation. 1x is stock. Push limits and the normal pushing speed ceiling still apply.",100.,0.25),
    number!("MaxPushDVStart","Push increase limit at low speed (m/s)","Caps the requested speed gain at low speed. Raising this alone does not make a push stronger; use Push strength.",100.,0.05),
    number!("MaxPushDVEnd","Push increase limit at high speed (m/s)","Caps the requested speed gain at high speed. Raising this alone does not make a push stronger; use Push strength.",100.,0.05),
    number!("PumpEffectFactor","Pumping acceleration strength","Scales speed gained by pumping transitions.",600.,0.5),
    number!("PumpEffectFactorAbsorption","Pumping speed absorption","Scales speed absorbed when compressing through transitions.",300.,0.25),
    number!("Hash_D77AFD320B6241C5","Pumping acceleration limit","Maximum acceleration per second allowed by the pumping controller.",400.,0.25),
    number!("Hash_09D1AEE3D7D8A4A7","Pumping absorption limit","Maximum speed absorption per second allowed by the pumping controller.",400.,0.25),
    number!("UnintentionalPumpScalar","Unintentional pumping strength","Scales pumping generated without the intentional-pump input.",30.,0.05),
    toggle!("JumpHeightOverrideEnabled","Difficulty jump-height override","Allow difficulty jump-height settings to override animation-provided heights."),
    number!("JumpMinHeight","Minimum regular jump height (m)","Lower height endpoint for regular jumps; speed curves also affect the result.",60.,0.05),
    number!("Hash_BE3F74F978D777E5","Minimum jump from manual (m)","Lower jump-height endpoint when the animation marks the trick as starting from a manual.",60.,0.05),
    number!("JumpMaxHeight","Maximum jump height (m)","Upper jump-height endpoint; speed curves also affect the result.",80.,0.05),
    number!("GrindLockDist","Grind targeting distance (m)","Distance used by airborne grind targeting. Larger values allow more assistance.",50.,0.05),
    number!("Hash_3097A69281990652","Standard grind pop - low strength","Lower vertical pop endpoint for 50-50, 5-0, backslash and darkslide exits.",100.,0.05),
    number!("Hash_FA4CDBAE0DFD1FAD","Standard grind pop - high strength","Upper vertical pop endpoint for 50-50, 5-0, backslash and darkslide exits.",100.,0.05),
    number!("Hash_B2B1170AFFC8AC69","Boardslide pop - low strength","Lower vertical pop endpoint when exiting a boardslide.",100.,0.05),
    number!("Hash_1B3E9F9C836D287D","Boardslide pop - high strength","Upper vertical pop endpoint when exiting a boardslide.",100.,0.05),
    number!("Hash_703829BD711E54DE","Tipslide pop - low strength","Lower vertical pop endpoint when exiting a nose/tail tipslide.",100.,0.05),
    number!("Hash_0F2473E9125079F0","Tipslide pop - high strength","Upper vertical pop endpoint when exiting a nose/tail tipslide.",100.,0.05),
    toggle!("EasyBodySpins","Assisted body spins","Enable the easier native airborne body-spin behavior."),
    toggle!("PerfectBodyFlips","Assisted body flips","Enable native perfect-flip assistance."),
    number!("MaxAutoBodySpinSpeed","Automatic body-spin speed limit","Maximum speed used by the automatic airborne body-spin calculation.",300.,0.25),
    number!("Hash_77AFCE78FE1206CA","Speed wobble onset","Speed threshold used to activate the board wobble system.",600.,0.5),
    number!("Hash_5B57F2CCCCEEF430","Speed wobble strength","Amplitude of board wobble above its activation threshold. Zero disables its amplitude.",30.,0.05),
    number!("Hash_84CB2F459F3FC811","Manual correction angular-speed threshold","Angular-speed threshold used by the manual balance correction calculation.",200.,0.1),
    toggle!("Hash_F8CBC0F5FEF2240E","Manual corrective force","Enable the native corrective-force branch during manuals."),
    toggle!("Hash_5548109D7B0CB70C","Forgiving surface response","Remap native surface categories using Easy's surface-assistance rule."),
    toggle!("WipeoutCheckForBadLanding","Bail on bad landings","Enable the bad-landing wipeout check."),
    number!("Hash_D978550D6DB4E6F7","Bad-landing check scale","Scales the native bad-landing check; this is not a general damage multiplier.",50.,0.05),
    number!("Wipeout_GroundXZAcceleration","Ground-impact bail threshold","Horizontal acceleration threshold used by ground wipeout checks.",400.,0.1),
    toggle!("Hash_CC890A3BDCF6290A","Bail when compressed","Enable the native squash/compression wipeout check."),
    toggle!("Hash_2B913C786BEFF4CD","Bail when falling upside down","Enable the airborne upside-down falling wipeout check."),
];

/// A named starting point for Custom: Easy's stock values with these fields
/// replaced. The motorized ones build on the stock Motorized mode (motor on,
/// no auto-push, no wobble amplitude, fewer bail checks) at different speeds.
pub(crate) struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub values: &'static [(&'static str, f32)],
}
const MOTOR_BASE: &[(&str, f32)] = &[
    ("MotorEnabled", 1.), ("AutoPushEnabled", 0.), ("PerfectBodyFlips", 0.),
    ("Hash_5B57F2CCCCEEF430", 0.), ("Hash_5548109D7B0CB70C", 0.), ("WipeoutCheckForBadLanding", 0.),
    ("UnintentionalPumpScalar", 0.7), ("JumpMinHeight", 1.33), ("Hash_BE3F74F978D777E5", 1.33), ("GrindLockDist", 0.9),
];
const NO_BAILS: &[(&str, f32)] = &[
    ("WipeoutCheckForBadLanding", 0.), ("Hash_CC890A3BDCF6290A", 0.), ("Hash_2B913C786BEFF4CD", 0.),
    ("Wipeout_GroundXZAcceleration", 400.),
];
pub(crate) const PRESETS: &[Preset] = &[
    Preset { name: "Motor Cruiser", description: "Motor on at an easy 40 km/h, no speed wobble.",
        values: &[("MotorTopSpeed", 40.)] },
    Preset { name: "Motor Street", description: "The stock Motorized mode: 90 km/h.",
        values: &[("MotorTopSpeed", 90.)] },
    Preset { name: "Motor Downhill", description: "120 km/h, with speed wobble and bad-landing bails back on.",
        values: &[("MotorTopSpeed", 120.), ("Hash_77AFCE78FE1206CA", 35.), ("Hash_5B57F2CCCCEEF430", 0.5), ("WipeoutCheckForBadLanding", 1.)] },
    Preset { name: "Motor Speed Demon", description: "160 km/h, wobble-free, longer grind targeting.",
        values: &[("MotorTopSpeed", 160.), ("Hash_77AFCE78FE1206CA", 80.), ("GrindLockDist", 2.)] },
    Preset { name: "Motor Insane", description: "300 km/h, big air, no bails. Hold on.",
        values: &[("MotorTopSpeed", 300.), ("Hash_77AFCE78FE1206CA", 150.), ("GrindLockDist", 3.),
            ("JumpMinHeight", 3.), ("JumpMaxHeight", 3.5), ("Hash_CC890A3BDCF6290A", 0.), ("Hash_2B913C786BEFF4CD", 0.),
            ("Wipeout_GroundXZAcceleration", 400.)] },
    Preset { name: "Chill Cruise", description: "Automatic, stronger pushes, forgiving landings and no bails.",
        values: &[("AutoPushEnabled", 1.), ("HostPushStrength", 1.5), ("Hash_5548109D7B0CB70C", 1.)] },
    Preset { name: "Big Air", description: "Higher pops and harder grind exits.",
        values: &[("JumpMinHeight", 2.6), ("Hash_BE3F74F978D777E5", 2.4), ("JumpMaxHeight", 3.2),
            ("Hash_3097A69281990652", 3.5), ("Hash_FA4CDBAE0DFD1FAD", 3.5), ("Hash_B2B1170AFFC8AC69", 3.5),
            ("Hash_1B3E9F9C836D287D", 3.5), ("Hash_703829BD711E54DE", 3.5), ("Hash_0F2473E9125079F0", 3.5)] },
    Preset { name: "Moon Jumps", description: "Low-gravity feel: very high pops and fast spins.",
        values: &[("JumpMinHeight", 5.), ("Hash_BE3F74F978D777E5", 5.), ("JumpMaxHeight", 6.), ("MaxAutoBodySpinSpeed", 12.),
            ("Hash_3097A69281990652", 5.), ("Hash_FA4CDBAE0DFD1FAD", 5.), ("Hash_B2B1170AFFC8AC69", 5.),
            ("Hash_1B3E9F9C836D287D", 5.), ("Hash_703829BD711E54DE", 5.), ("Hash_0F2473E9125079F0", 5.)] },
    Preset { name: "Grind Magnet", description: "Rails pull you in from much further away.",
        values: &[("GrindLockDist", 3.), ("Hash_3097A69281990652", 3.), ("Hash_FA4CDBAE0DFD1FAD", 3.)] },
    Preset { name: "Flip Master", description: "Assisted spins and perfect flips, faster rotation.",
        values: &[("EasyBodySpins", 1.), ("PerfectBodyFlips", 1.), ("MaxAutoBodySpinSpeed", 14.)] },
    Preset { name: "Pump Track", description: "Pumping through transitions builds serious speed.",
        values: &[("PumpEffectFactor", 40.), ("Hash_D77AFD320B6241C5", 20.), ("UnintentionalPumpScalar", 1.5)] },
    Preset { name: "No Bails", description: "Every wipeout check off.", values: NO_BAILS },
    Preset { name: "Sim", description: "Hardcore physics with an earlier speed wobble.",
        values: &[("AutoPushEnabled", 0.), ("MaxPushDVStart", 0.5), ("MaxPushDVEnd", 0.25), ("PumpEffectFactorAbsorption", 9.),
            ("Hash_D77AFD320B6241C5", 8.), ("Hash_09D1AEE3D7D8A4A7", 8.), ("UnintentionalPumpScalar", 0.), ("JumpHeightOverrideEnabled", 0.),
            ("JumpMinHeight", 1.21), ("GrindLockDist", 0.15), ("EasyBodySpins", 0.), ("PerfectBodyFlips", 0.), ("MaxAutoBodySpinSpeed", 0.5),
            ("Hash_77AFCE78FE1206CA", 10.), ("Hash_5B57F2CCCCEEF430", 1.), ("Hash_F8CBC0F5FEF2240E", 1.), ("Hash_5548109D7B0CB70C", 0.),
            ("Hash_CC890A3BDCF6290A", 1.), ("Hash_2B913C786BEFF4CD", 1.), ("Wipeout_GroundXZAcceleration", 7.1)] },
    Preset { name: "Arcade", description: "Big air, grind magnet, flip assists, auto-push and no bails together.",
        values: &[("AutoPushEnabled", 1.), ("JumpMinHeight", 2.6), ("JumpMaxHeight", 3.2), ("GrindLockDist", 3.),
            ("EasyBodySpins", 1.), ("PerfectBodyFlips", 1.), ("MaxAutoBodySpinSpeed", 14.),
            ("WipeoutCheckForBadLanding", 0.), ("Hash_CC890A3BDCF6290A", 0.), ("Hash_2B913C786BEFF4CD", 0.),
            ("Wipeout_GroundXZAcceleration", 400.)] },
];
impl Preset {
    /// Easy's values (`defaults`), then the motorized base if this is a motor
    /// preset, then this preset's own fields.
    pub fn tuning(&self, defaults: &Tuning) -> Tuning {
        let mut tuning = defaults.clone();
        let motor = self.values.iter().any(|(k, _)| *k == "MotorTopSpeed");
        let chill = self.name == "Chill Cruise";
        for (key, value) in MOTOR_BASE.iter().filter(|_| motor).chain(NO_BAILS.iter().filter(|_| chill)).chain(self.values) {
            tuning.values.insert((*key).into(), *value);
        }
        tuning
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tuning { pub values: BTreeMap<String, f32> }
impl Tuning {
    pub fn defaults(data: &Collections) -> Result<Self,String> {
        let mut values=BTreeMap::new();
        for o in OPTIONS {
            let v=if o.key=="HostPushStrength" {1.} else if o.boolean { u8::from(data.boolean("physics_mode","easy",o.key)?) as f32 }
                else {data.float("physics_mode","easy",o.key)?};
            values.insert(o.key.into(),v);
        }
        Ok(Self {values})
    }
    pub fn load(root: &Path, data: &Collections) -> Result<Self,String> {
        let mut result=Self::defaults(data)?;
        let path=crate::difficulty::Difficulty::path(root).with_file_name("custom-difficulty.json");
        match skate_data::vfs::read(&path) {
            Ok(bytes)=> {let saved:Self=serde_json::from_slice(&bytes).map_err(|e|format!("Custom difficulty: {e}"))?;
                saved.validate()?; result.values.extend(saved.values);},
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
            Err(e)=>return Err(format!("{}: {e}",path.display())),
        }
        Ok(result)
    }
    pub fn validate(&self)->Result<(),String> {
        for (key,&value) in &self.values {
            let o=OPTIONS.iter().find(|o|o.key==key).ok_or_else(||format!("Unknown custom difficulty field: {key}"))?;
            if !value.is_finite() || value<0. || value>o.max || (o.boolean && value!=0. && value!=1.) {
                return Err(format!("Invalid value for {}",o.label));
            }
        }
        Ok(())
    }
    pub fn save(&self,root:&Path)->Result<(),String> {
        self.validate()?;
        let path=crate::difficulty::Difficulty::path(root).with_file_name("custom-difficulty.json");
        skate_data::vfs::create_dir_all(path.parent().unwrap()).map_err(|e|e.to_string())?;
        let temp=path.with_extension("json.tmp");
        skate_data::vfs::write(&temp,serde_json::to_vec_pretty(self).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        skate_data::vfs::rename(temp,path).map_err(|e|e.to_string())
    }
    pub fn overlay(&self,data:&mut Collections)->Result<(),String> {
        self.validate()?;
        let fields=self.values.iter().map(|(key,&v)| {
            let o=OPTIONS.iter().find(|o|o.key==key).unwrap();
            (key.clone(),Field {type_name:if o.boolean {"EA::Reflection::Bool"}else{"EA::Reflection::Float"}.into(),
                data:if o.boolean {format!("{:02X}",v as u8)}else{format!("{:08X}",v.to_bits())}})
        }).collect();
        data.override_profile("physics_mode","test","easy",fields)
    }
    pub fn value(&self,index:usize)->f32 {self.values.get(OPTIONS[index].key).copied().unwrap_or(0.)}
    pub fn adjust(&mut self,index:usize,direction:i32) {
        let o=&OPTIONS[index];let v=self.value(index);
        let v=if o.boolean {1.-v} else {(v+direction as f32*o.step).clamp(0.,o.max)};
        self.values.insert(o.key.into(),v);
    }
    pub fn set_fraction(&mut self,index:usize,fraction:f32) {
        let o=&OPTIONS[index];let v=((fraction.clamp(0.,1.)*o.max/o.step).round()*o.step).clamp(0.,o.max);
        self.values.insert(o.key.into(),v);
    }
    pub fn label(&self,index:usize)->String {
        let o=&OPTIONS[index];let v=self.value(index);
        if o.boolean {format!("{}     {}",o.label,if v!=0. {"On"}else{"Off"})}
        else {format!("{}     {:.2}",o.label,v)}
    }
}
pub(crate) fn load_collections(root:&Path)->Result<Collections,String> {
    let mut data=Collections::load(root)?;
    Tuning::load(root,&data)?.overlay(&mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_custom_values_are_rejected() {
        for v in [f32::NAN,-1.,2001.] {
            let mut t=Tuning::default();t.values.insert("MotorTopSpeed".into(),v);assert!(t.validate().is_err());
        }
        let mut t=Tuning::default();t.values.insert("MotorEnabled".into(),0.5);assert!(t.validate().is_err());
    }
    #[test]
    fn custom_inherits_easy_without_changing_stock_or_assets() {
        let mut data:Collections=serde_json::from_value(serde_json::json!({"version":1,"collections":[
            {"class":"physics_mode","key":"easy","parent":"","source":"test","sha256":"","fields":{
                "MotorEnabled":{"type":"EA::Reflection::Bool","data":"00"},
                "MotorTopSpeed":{"type":"EA::Reflection::Float","data":"00000000"}}}
        ]})).unwrap();
        let mut t=Tuning::default();t.values.insert("MotorTopSpeed".into(),90.);t.overlay(&mut data).unwrap();
        assert_eq!(data.float("physics_mode","test","MotorTopSpeed").unwrap(),90.);
        assert!(!data.boolean("physics_mode","test","MotorEnabled").unwrap());
        assert_eq!(data.float("physics_mode","easy","MotorTopSpeed").unwrap(),0.);
    }
}


#[cfg(test)]
mod stock_tests {
    use super::*;
    #[test]
    #[ignore = "requires private stock collections"]
    fn every_stock_difficulty_field_has_a_control_and_easy_default() {
        let root=std::env::var_os("SKATE3_ASSET_ROOT").unwrap();
        let mut data=Collections::load(Path::new(&root)).unwrap();
        let tuning=Tuning::defaults(&data).unwrap(); tuning.validate().unwrap();
        let ids:std::collections::BTreeSet<_>=OPTIONS.iter().map(|o|skate_data::attrib_hash::numeric_name(o.key)).collect();
        assert_eq!(ids.len(),OPTIONS.len());
        for c in data.entries().iter().filter(|c|c.class_name=="physics_mode") {
            for key in c.fields.keys() {assert!(ids.contains(&skate_data::attrib_hash::numeric_name(key)),"Missing control for {key}");}
        }
        tuning.overlay(&mut data).unwrap();
        for o in OPTIONS.iter().filter(|o|o.key!="HostPushStrength") { assert_eq!(data.field("physics_mode","easy",o.key).unwrap().data,
            data.field("physics_mode","test",o.key).unwrap().data, "{}",o.label); }
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    #[test]
    fn every_preset_is_valid_and_names_real_fields() {
        let mut defaults = Tuning::default();
        for o in OPTIONS { defaults.values.insert(o.key.into(), 0.); }
        for preset in PRESETS {
            for (key, _) in preset.values {
                assert!(OPTIONS.iter().any(|o| o.key == *key), "{}: unknown field {key}", preset.name);
            }
            preset.tuning(&defaults).validate().unwrap_or_else(|e| panic!("{}: {e}", preset.name));
        }
        let motor = PRESETS.iter().find(|p| p.name == "Motor Speed Demon").unwrap().tuning(&defaults);
        assert_eq!(motor.values["MotorEnabled"], 1.);
        assert_eq!(motor.values["MotorTopSpeed"], 160.);
    }
}

#[cfg(test)]
mod push_slider_tests {
    use super::*;
    #[test]
    fn strength_adjustment_and_saved_values_round_trip() {
        let i=OPTIONS.iter().position(|o|o.key=="HostPushStrength").unwrap();
        let mut t=Tuning::default();t.values.insert("HostPushStrength".into(),1.);
        t.adjust(i,1);assert_eq!(t.value(i),1.25);
        t.set_fraction(i,0.5);assert_eq!(t.value(i),50.);
        let saved=serde_json::to_vec(&t).unwrap();
        let loaded:Tuning=serde_json::from_slice(&saved).unwrap();assert_eq!(loaded.value(i),50.);
    }
}