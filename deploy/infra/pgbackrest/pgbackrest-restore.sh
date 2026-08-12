#!/usr/bin/env bash
set -Eeuo pipefail

# 此一次性任务将物理备份写回正式 PostgreSQL 数据卷。它不会替运维人员停库，也不会
# 启停正式数据库；恢复后的 WAL/PITR 回放由随后启动的正式 db 容器完成。
readonly STANZA="qddxp"
readonly RESTORE_PGDATA="/data/db/18/docker"
readonly POSTGRES_HOST="db"
readonly POSTGRES_PORT="5432"
readonly RESTORE_TYPE="${PGBR_RESTORE_TYPE:-default}"
readonly RESTORE_TARGET="${PGBR_RESTORE_TARGET:-}"
readonly RESTORE_SET="${PGBR_RESTORE_SET:-latest}"

log() {
    local level="$1"
    shift
    printf 'timestamp=%s level=%s component=pgbackrest-restore message="%s"\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$level" "$*"
}

on_error() {
    local exit_code=$?
    log ERROR "恢复任务异常退出 exit_code=${exit_code} line=${BASH_LINENO[0]} command=${BASH_COMMAND}"
    exit "$exit_code"
}

trap on_error ERR
trap 'log INFO "收到停止信号，恢复任务退出"; exit 0' TERM INT

case "$RESTORE_TYPE" in
    default|immediate|time|lsn|name|xid)
        ;;
    *)
        log ERROR "不支持的 PGBR_RESTORE_TYPE=${RESTORE_TYPE}"
        exit 1
        ;;
esac

if [[ "$RESTORE_TYPE" == "time" || "$RESTORE_TYPE" == "lsn" || \
      "$RESTORE_TYPE" == "name" || "$RESTORE_TYPE" == "xid" ]]; then
    if [[ -z "$RESTORE_TARGET" ]]; then
        log ERROR "恢复类型 ${RESTORE_TYPE} 必须同时设置 PGBR_RESTORE_TARGET"
        exit 1
    fi
elif [[ -n "$RESTORE_TARGET" ]]; then
    log ERROR "恢复类型 ${RESTORE_TYPE} 不接受 PGBR_RESTORE_TARGET"
    exit 1
fi

# pg_isready 返回 0 表示正在接受连接，1 表示服务器正在启动、关闭或恢复；两者都说明
# PostgreSQL 进程仍存在。只有返回 2（没有服务器响应）时才允许继续恢复。
postgres_status=0
pg_isready --quiet --host="$POSTGRES_HOST" --port="$POSTGRES_PORT" --timeout=3 \
    || postgres_status=$?

case "$postgres_status" in
    0|1)
        log ERROR "检测到 PostgreSQL 仍在运行，请先停止数据库"
        log ERROR "请由运维人员执行：docker compose -f deploy/compose.yml stop qddxp pgbackrest-backup db"
        exit 1
        ;;
    2)
        log INFO "未检测到运行中的 PostgreSQL，继续检查正式数据目录"
        ;;
    *)
        log ERROR "无法确认 PostgreSQL 是否已停止，pg_isready exit_code=${postgres_status}；为避免损坏数据，拒绝恢复"
        exit 1
        ;;
esac

mkdir -p "$RESTORE_PGDATA"
if [[ -n "$(find "$RESTORE_PGDATA" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    log ERROR "正式 PGDATA 不是空目录；为避免覆盖现场或混合数据，拒绝恢复 ${RESTORE_PGDATA}"
    log ERROR "请由运维人员按 README 备份或移走原 PGDATA，确认目录为空后重试"
    exit 1
fi

restore_args=(
    --stanza="$STANZA"
    --pg1-path="$RESTORE_PGDATA"
    --type="$RESTORE_TYPE"
)

# pgBackRest 用“不传 --set”表示最新备份；latest 并不是合法的实际备份标签。
if [[ "$RESTORE_SET" != "latest" ]]; then
    restore_args+=(--set="$RESTORE_SET")
fi

if [[ -n "$RESTORE_TARGET" ]]; then
    restore_args+=(
        --target="$RESTORE_TARGET"
        --target-action=promote
        --target-timeline=latest
    )
elif [[ "$RESTORE_TYPE" == "immediate" ]]; then
    restore_args+=(
        --target-action=promote
        --target-timeline=latest
    )
fi

log INFO "开始恢复 backup_set=${RESTORE_SET} type=${RESTORE_TYPE} target=${RESTORE_TARGET:-全部可用WAL}"
pgbackrest "${restore_args[@]}" restore
log INFO "物理文件恢复完成；请启动正式 db 容器，并观察日志确认 WAL/PITR 回放完成"
