// MI_SpawnMods.SpawnMods spawns every mod's InitSpacerig / InitCave with a ForEach over the pure
// GetInitActors: an AssetRegistry GetAssetsByClass("BlueprintGeneratedClass") filtered by name. The
// compiled ForEach calls it again at the loop head and in the body, so the list is re-queried on every
// iteration. UE lists in-memory classes before on-disk rows, and each init the loop loads moves from one
// part to the other, so the list reorders under the index and some inits are never visited. A 180-mod
// profile never started the mod at #153 for this reason.
//
// The patch keeps the first query. The loop's back-edge jumps past the head's re-query, and the body's
// re-query becomes no-ops of the same memory size, so no other jump offset moves.
use super::*;
use unreal_asset::exports::Export;
use unreal_asset::kismet::{ExJump, ExNothing};

pub(crate) const SPAWN_MODS_PATH: &str = "FSD/Content/_AssemblyStorm/ModIntegration/MI_SpawnMods";

// Memory offsets in the bundled MI_SpawnMods: the loop head the back-edge jumps to, the statement right
// after the head's GetInitActors query, and the memory size of the body's query statement.
// ponytail: pinned to the bundled asset; the test below fails if it is ever rebuilt.
const LOOP_HEAD: u32 = 111;
const AFTER_QUERY: u32 = 362;
const QUERY_SIZE: usize = 85;

fn spawn_mods_code<C: Read + Seek>(asset: &mut Asset<C>) -> Option<&mut Vec<KismetExpression>> {
    asset.asset_data.exports.iter_mut().find_map(|e| match e {
        Export::FunctionExport(f)
            if f.get_base_export()
                .object_name
                .get_content(|n| n == "SpawnMods") =>
        {
            f.struct_export.script_bytecode.as_mut()
        }
        _ => None,
    })
}

fn init_actor_queries(code: &[KismetExpression]) -> Vec<usize> {
    let is_query = |ex: &KismetExpression| {
        matches!(ex, KismetExpression::ExLet(l)
            if matches!(&*l.expression, KismetExpression::ExLocalVirtualFunction(f)
                if f.virtual_function_name.get_content(|n| n == "GetInitActors")))
    };
    (0..code.len()).filter(|&i| is_query(&code[i])).collect()
}

pub(crate) fn patch_spawn_mods<C: Read + Seek>(
    asset: &mut Asset<C>,
) -> Result<(), IntegrationError> {
    let fail = |what: &str| IntegrationError::GenericError {
        msg: format!("MI_SpawnMods.SpawnMods is not the bundled shape: {what}"),
    };
    let code = spawn_mods_code(asset).ok_or_else(|| fail("no SpawnMods bytecode"))?;
    let &[_, body] = init_actor_queries(code).as_slice() else {
        return Err(fail("expected two GetInitActors queries"));
    };
    let back_edge = code
        .iter()
        .rposition(|ex| matches!(ex, KismetExpression::ExJump(j) if j.code_offset == LOOP_HEAD))
        .ok_or_else(|| fail("no back-edge to the loop head"))?;

    code[back_edge] = ExJump {
        token: EExprToken::ExJump,
        code_offset: AFTER_QUERY,
    }
    .into();
    code.splice(
        body..=body,
        std::iter::repeat_with(|| ExNothing::default().into()).take(QUERY_SIZE),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UASSET: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/integration/FSD/Content/_AssemblyStorm/ModIntegration/MI_SpawnMods.uasset"
    ));
    const UEXP: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/integration/FSD/Content/_AssemblyStorm/ModIntegration/MI_SpawnMods.uexp"
    ));

    fn read(uasset: &[u8], uexp: &[u8]) -> Asset<Cursor<Vec<u8>>> {
        AssetBuilder::new(Cursor::new(uasset.to_vec()), EngineVersion::VER_UE4_27)
            .bulk(Cursor::new(uexp.to_vec()))
            .build()
            .unwrap()
    }

    fn bytecode_size(asset: &Asset<Cursor<Vec<u8>>>) -> i32 {
        asset
            .asset_data
            .exports
            .iter()
            .find_map(|e| match e {
                Export::FunctionExport(f)
                    if f.get_base_export()
                        .object_name
                        .get_content(|n| n == "SpawnMods") =>
                {
                    Some(f.struct_export.script_bytecode_size)
                }
                _ => None,
            })
            .unwrap()
    }

    // The patched SpawnMods round-trips with the same memory size (so every jump still lands on its
    // statement), queries GetInitActors once, and loops back to the statement after that query.
    #[test]
    fn spawn_mods_queries_init_actors_once() {
        let mut asset = read(UASSET, UEXP);
        let before = bytecode_size(&asset);
        patch_spawn_mods(&mut asset).unwrap();

        let (mut uasset, mut uexp) = (Cursor::new(vec![]), Cursor::new(vec![]));
        asset.write_data(&mut uasset, Some(&mut uexp)).unwrap();
        let mut again = read(uasset.get_ref(), uexp.get_ref());

        assert_eq!(bytecode_size(&again), before);
        let code = spawn_mods_code(&mut again).unwrap();
        assert_eq!(init_actor_queries(code).len(), 1);
        assert!(
            code.iter().any(
                |ex| matches!(ex, KismetExpression::ExJump(j) if j.code_offset == AFTER_QUERY)
            )
        );
        assert!(
            !code
                .iter()
                .any(|ex| matches!(ex, KismetExpression::ExJump(j) if j.code_offset == LOOP_HEAD))
        );
    }
}
