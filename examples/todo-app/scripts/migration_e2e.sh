#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd -- "${EXAMPLE_DIR}/../.." && pwd)"
WORK_ROOT="${1:-$(mktemp -d "${TMPDIR:-/tmp}/sql-orm-todo-migrations.XXXXXX")}"
CLI_BIN="${REPO_ROOT}/target/debug/sql-orm-cli"
MANIFEST_PATH="${EXAMPLE_DIR}/Cargo.toml"

cargo build --manifest-path "${REPO_ROOT}/crates/sql-orm-cli/Cargo.toml" >/dev/null

mkdir -p "${WORK_ROOT}"

(
    cd "${WORK_ROOT}"
    "${CLI_BIN}" migration add CreateTodoSchema \
        --snapshot-bin model_snapshot \
        --manifest-path "${MANIFEST_PATH}"
    "${CLI_BIN}" migration add VerifyTodoSchemaNoop \
        --snapshot-bin model_snapshot \
        --manifest-path "${MANIFEST_PATH}"
    "${CLI_BIN}" database update > database_update.sql
)

INITIAL_MIGRATION_DIR="$(find "${WORK_ROOT}/migrations" -maxdepth 1 -type d -name '*_createtodoschema' | sort | tail -n 1)"
if [[ -z "${INITIAL_MIGRATION_DIR}" ]]; then
    printf 'Expected initial migration directory was not generated.\n' >&2
    exit 1
fi

for expected in \
    '"name": "audit_events"' \
    '"name": "created_at"' \
    '"name": "created_by_user_id"' \
    '"name": "updated_at"' \
    '"name": "updated_by"'
do
    if ! grep -Fq "${expected}" "${INITIAL_MIGRATION_DIR}/model_snapshot.json"; then
        printf 'Expected %s in %s/model_snapshot.json\n' "${expected}" "${INITIAL_MIGRATION_DIR}" >&2
        exit 1
    fi
done

if ! grep -Fq 'CREATE TABLE [todo].[audit_events]' "${INITIAL_MIGRATION_DIR}/up.sql"; then
    printf 'Expected audit_events table DDL in %s/up.sql\n' "${INITIAL_MIGRATION_DIR}" >&2
    exit 1
fi

printf 'Migration workspace: %s\n' "${WORK_ROOT}"
printf 'Generated script: %s\n' "${WORK_ROOT}/database_update.sql"
printf 'Validated audit snapshot: %s/model_snapshot.json\n' "${INITIAL_MIGRATION_DIR}"

if [[ -n "${SQL_ORM_SQLCMD_SERVER:-}" && -n "${SQL_ORM_SQLCMD_USER:-}" && -n "${SQL_ORM_SQLCMD_PASSWORD:-}" ]]; then
    sqlcmd -S "${SQL_ORM_SQLCMD_SERVER}" \
        -U "${SQL_ORM_SQLCMD_USER}" \
        -P "${SQL_ORM_SQLCMD_PASSWORD}" \
        -d "${SQL_ORM_SQLCMD_DATABASE:-tempdb}" \
        -C -b -i "${WORK_ROOT}/database_update.sql"
else
    printf 'SQL_ORM_SQLCMD_SERVER/USER/PASSWORD are not set; SQL Server apply step was skipped.\n'
fi
