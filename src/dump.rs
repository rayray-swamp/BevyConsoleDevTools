//! State dumps reuse BRP's builtin reflection handlers (same output as `world.query`).

use bevy::ecs::system::In;
use bevy::prelude::World;
use bevy::remote::builtin_methods::{process_remote_get_resources_request, process_remote_query_request};
use serde_json::{Map, Value, json};

pub fn dump_json(world: &mut World, resources: &[String], components: &[String]) -> Result<Value, String> {
    let mut res = Map::new();
    for r in resources {
        let v = process_remote_get_resources_request(In(Some(json!({ "resource": r }))), world)
            .map_err(|e| format!("resource {r}: {}", e.message))?;
        res.insert(r.clone(), v);
    }
    let mut comps = Map::new();
    for c in components {
        // strict: a misspelled type must fail instead of silently returning nothing
        let v = process_remote_query_request(
            In(Some(json!({ "data": { "components": [c] }, "strict": true }))),
            world,
        )
        .map_err(|e| format!("component {c}: {}", e.message))?;
        comps.insert(c.clone(), v);
    }
    Ok(json!({ "resources": res, "components": comps }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use bevy::reflect::TypePath;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct Foo {
        v: i32,
    }

    #[test]
    fn dump_component_values() {
        let mut app = App::new();
        app.register_type::<Foo>();
        app.world_mut().spawn(Foo { v: 7 });
        let v = dump_json(app.world_mut(), &[], &[Foo::type_path().to_string()]).unwrap();
        assert!(v.to_string().contains("\"v\":7"), "{v}");
    }

    #[test]
    fn dump_unknown_component_is_error() {
        let mut app = App::new();
        assert!(dump_json(app.world_mut(), &[], &["nope::Missing".into()]).is_err());
    }
}
