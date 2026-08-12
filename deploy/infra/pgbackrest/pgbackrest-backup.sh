#!/usr/bin/env bash
set -Eeuo pipefail

# 此脚本在 Percona PostgreSQL/pgBackRest 原始镜像中运行，不构建定制镜像。
# 调度标记随备份仓库持久化；容器错过计划时刻后，会补做当前周期缺失的备份。
readonly STANZA="qddxp"
readonly PG_SOCKET_PATH="/var/run/postgresql"
readonly PG_USER="${POSTGRES_USER:-postgres}"
readonly PG_DATABASE="${POSTGRES_DB:-qddxp}"
readonly SCHEDULE_PATH="/backrestrepo/.qddxp-schedule"
readonly SCHEDULE_HOUR_UTC="${PGBR_SCHEDULE_HOUR_UTC:-3}"
readonly CHECK_INTERVAL_SECONDS="${PGBR_CHECK_INTERVAL_SECONDS:-300}"

log() {
    local level="$1"
    shift
    printf 'timestamp=%s level=%s component=pgbackrest-backup message="%s"\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$level" "$*"
}

on_error() {
    local exit_code=$?
    log ERROR "备份调度器异常退出 exit_code=${exit_code} line=${BASH_LINENO[0]} command=${BASH_COMMAND}"
    exit "$exit_code"
}

trap on_error ERR
trap 'log INFO "收到停止信号，备份调度器退出"; exit 0' TERM INT

if [[ ! "$SCHEDULE_HOUR_UTC" =~ ^([0-9]|1[0-9]|2[0-3])$ ]]; then
    log ERROR "PGBR_SCHEDULE_HOUR_UTC 必须是 0 到 23 的整数"
    exit 1
fi

if [[ ! "$CHECK_INTERVAL_SECONDS" =~ ^[1-9][0-9]*$ ]]; then
    log ERROR "PGBR_CHECK_INTERVAL_SECONDS 必须是正整数"
    exit 1
fi

readonly SCHEDULE_HOUR_NUMBER="$((10#$SCHEDULE_HOUR_UTC))"

mkdir -p "$SCHEDULE_PATH"

log INFO "等待 PostgreSQL Unix Socket 就绪 database=${PG_DATABASE} user=${PG_USER}"
until pg_isready --quiet \
    --host="$PG_SOCKET_PATH" \
    --username="$PG_USER" \
    --dbname="$PG_DATABASE"; do
    sleep 5
done

log INFO "PostgreSQL 已就绪，初始化并检查 pgBackRest stanza"
pgbackrest --stanza="$STANZA" stanza-create
pgbackrest --stanza="$STANZA" check
touch /tmp/pgbackrest-backup-ready
log INFO "pgBackRest stanza 检查通过，备份调度器已就绪"

run_backup() {
    local backup_type="$1"
    local full_marker="$2"
    local daily_marker="$3"
    local marker_tmp

    log INFO "开始执行 ${backup_type} 备份"
    if ! pgbackrest --stanza="$STANZA" --type="$backup_type" backup; then
        log ERROR "${backup_type} 备份失败，将在下个检查周期重试"
        return 1
    fi

    # 仅在 pgBackRest 成功返回后原子写入标记，防止中断的备份被误判为已完成。
    marker_tmp="${daily_marker}.tmp.$$"
    date -u +%Y-%m-%dT%H:%M:%SZ > "$marker_tmp"
    mv "$marker_tmp" "$daily_marker"

    if [[ "$backup_type" == "full" ]]; then
        marker_tmp="${full_marker}.tmp.$$"
        date -u +%Y-%m-%dT%H:%M:%SZ > "$marker_tmp"
        mv "$marker_tmp" "$full_marker"
    fi

    log INFO "${backup_type} 备份成功"
}

while true; do
    today="$(date -u +%F)"
    current_hour="$((10#$(date -u +%H)))"
    weekday="$((10#$(date -u +%w)))"
    now_epoch="$(date -u +%s)"

    # 周期以周日为首日；容器停机错过计划时刻后，会补做当前周期的 full。
    full_period_epoch="$((now_epoch - weekday * 86400))"
    full_period="$(date -u --date="@${full_period_epoch}" +%F)"
    full_marker="${SCHEDULE_PATH}/full-${full_period}"
    daily_marker="${SCHEDULE_PATH}/daily-${today}"

    # 全新仓库不等待计划时刻，立即建立第一条可恢复的 full 备份链。
    if [[ -z "$(find "$SCHEDULE_PATH" -maxdepth 1 -type f -name 'full-*' -print -quit)" ]]; then
        run_backup full "$full_marker" "$daily_marker" || true
    elif (( current_hour >= SCHEDULE_HOUR_NUMBER )); then
        if [[ ! -f "$full_marker" ]]; then
            run_backup full "$full_marker" "$daily_marker" || true
        elif [[ ! -f "$daily_marker" ]]; then
            run_backup diff "$full_marker" "$daily_marker" || true
        fi
    fi

    sleep "$CHECK_INTERVAL_SECONDS"
done
