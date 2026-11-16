// Mechanical source extraction from src/integrate.rs; no algorithm changes.
use super::*;

pub(crate) fn patch<C: Seek + Read>(asset: &mut Asset<C>) -> Result<(), IntegrationError> {
    let ver = AssetVersion::new_from(asset);
    let mut statements = extract_tracked_statements(asset, ver, &None);

    let find_function = |name| {
        asset
            .imports
            .iter()
            .enumerate()
            .find(|(_, i)| {
                i.class_package.get_content(|s| s == "/Script/CoreUObject")
                    && i.class_name.get_content(|s| s == "Function")
                    && i.object_name.get_content(|s| s == name)
            })
            .map(|(pi, _)| PackageIndex::from_import(pi as i32).unwrap())
    };

    fn patch_ismodded(
        is_modded: Option<PackageIndex>,
        is_modded_sandbox: Option<PackageIndex>,
        mut statement: TrackedStatement,
    ) -> Option<TrackedStatement> {
        walk(&mut statement.ex, &|ex| {
            if let KismetExpression::ExCallMath(f) = ex
                && (Some(f.stack_node) == is_modded || Some(f.stack_node) == is_modded_sandbox)
            {
                *ex = ExFalse::default().into()
            }
        });
        Some(statement)
    }

    let is_modded = find_function("FSDIsModdedServer");
    let is_modded_sandbox = find_function("FSDIsModdedSandboxServer");

    for statements in statements.values_mut() {
        *statements = std::mem::take(statements)
            .into_iter()
            .filter_map(|s| patch_ismodded(is_modded, is_modded_sandbox, s))
            .collect();
    }
    inject_tracked_statements(asset, ver, statements);
    Ok(())
}
