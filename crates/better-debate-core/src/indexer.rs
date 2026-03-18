use crate::runtime::AppHandle;

use crate::db::open_database;
use crate::lexical;
use crate::query_engine;
use crate::CommandResult;

pub(crate) fn rebuild_lexical_index(app: &AppHandle) -> CommandResult<()> {
    let connection = open_database(app)?;
    lexical::replace_all_documents_from_connection(app, &connection)?;
    query_engine::clear_query_cache();
    Ok(())
}

pub(crate) fn rebuild_lexical_index_for_root(app: &AppHandle, root_id: i64) -> CommandResult<()> {
    rebuild_lexical_index_for_root_with_options(app, root_id, true)
}

pub(crate) fn rebuild_lexical_index_for_root_with_options(
    app: &AppHandle,
    root_id: i64,
    wait_for_merges: bool,
) -> CommandResult<()> {
    let connection = open_database(app)?;
    lexical::replace_root_documents_from_connection_with_options(
        app,
        &connection,
        root_id,
        wait_for_merges,
    )?;
    query_engine::clear_query_cache();
    Ok(())
}

pub(crate) fn apply_lexical_index_file_changes_for_root_with_options(
    app: &AppHandle,
    root_id: i64,
    updated_file_ids: &[i64],
    removed_file_ids: &[i64],
    wait_for_merges: bool,
) -> CommandResult<()> {
    let connection = open_database(app)?;
    lexical::apply_file_changes_from_connection_with_options(
        app,
        &connection,
        root_id,
        updated_file_ids,
        removed_file_ids,
        wait_for_merges,
    )?;
    query_engine::clear_query_cache();
    Ok(())
}
