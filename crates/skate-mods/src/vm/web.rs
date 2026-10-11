use super::*;

pub const LUA_INSTRUCTIONS_PER_BUDGET_UNIT: usize = 1000;
pub const LUA_BUDGET_UNITS: usize = 800;

/// Lua mods need a C Lua build that does not exist for wasm32-unknown-unknown.
pub struct Vm;

impl Vm {
    pub fn new(
        _root: &Path,
        _manifest: &Manifest,
        _settings: &BTreeMap<String, Value>,
        _snapshot: &Value,
    ) -> Result<Self, String> {
        Err("Lua mods are unavailable in the web build".into())
    }

    #[allow(dead_code)]
    pub fn settings(&mut self, _settings: &BTreeMap<String, Value>) -> Result<(), String> {
        Ok(())
    }

    pub fn call_shared(
        &mut self,
        _name: &str,
        _payload: Value,
        _snapshot: &Arc<Value>,
        _physics: Option<Value>,
        _fields: &crate::SnapshotFields,
    ) -> Result<Vec<Command>, String> {
        Ok(Vec::new())
    }
}
