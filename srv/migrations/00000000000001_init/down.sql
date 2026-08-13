-- 按创建顺序逆序删除全部业务表。表之间没有外键，顺序主要用于保持迁移结构清晰。
DROP TABLE IF EXISTS notification_outbox;
DROP TABLE IF EXISTS api_call_logs;
DROP TABLE IF EXISTS payment_events;
DROP TABLE IF EXISTS payment_attempts;
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS products;
DROP TABLE IF EXISTS product_info;

-- pgcrypto 可能由同一数据库中的其他应用预先安装，因此回滚时不删除共享扩展。
