-- V6__bind_algorithm_instances_to_tasks.sql
-- 绑定 algorithm_instances 到 analysis_tasks，建立 NOT NULL + UNIQUE(task_id, algorithm_id) 一对多关系

-- 1. 重建 algorithm_instances 为任务专属表，并写入干净迁移数据。
DROP TABLE IF EXISTS algorithm_instances_new;
CREATE TABLE algorithm_instances_new (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id    TEXT NOT NULL,
    task_id        INTEGER NOT NULL,
    camera_id      TEXT NOT NULL,
    algorithm_id   TEXT NOT NULL,
    analysis_fps   INTEGER NOT NULL DEFAULT 0,
    params_json    TEXT NOT NULL DEFAULT '{}',
    rules_json     TEXT NOT NULL DEFAULT '[]',
    motion_gate_json TEXT NOT NULL DEFAULT '{}',
    enabled        INTEGER NOT NULL DEFAULT 0,
    actual_status  INTEGER NOT NULL DEFAULT 0,
    status_message TEXT NOT NULL DEFAULT '',
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(task_id) REFERENCES analysis_tasks(id) ON DELETE CASCADE
);

-- 2. 只迁移有对应任务的旧实例；同一任务重复算法保留 id 最小的一条，其余清理。
INSERT INTO algorithm_instances_new (
    instance_id,
    task_id,
    camera_id,
    algorithm_id,
    analysis_fps,
    params_json,
    rules_json,
    motion_gate_json,
    enabled,
    actual_status,
    status_message,
    created_at,
    updated_at
)
SELECT
    old.instance_id,
    t.id AS task_id,
    old.camera_id,
    old.algorithm_id,
    old.analysis_fps,
    old.params_json,
    old.rules_json,
    old.motion_gate_json,
    old.enabled,
    CASE
        WHEN old.algorithm_id IN (SELECT algorithm_id FROM algorithms) THEN old.actual_status
        ELSE 5
    END,
    CASE
        WHEN old.algorithm_id IN (SELECT algorithm_id FROM algorithms) THEN old.status_message
        ELSE '算法包数据校验缺失，已标记为不可用状态'
    END,
    old.created_at,
    old.updated_at
FROM algorithm_instances AS old
INNER JOIN analysis_tasks AS t ON t.camera_id = old.camera_id
WHERE old.id IN (
    SELECT MIN(o.id)
    FROM algorithm_instances AS o
    INNER JOIN analysis_tasks AS tt ON tt.camera_id = o.camera_id
    GROUP BY o.camera_id, o.algorithm_id
);

-- 3. 删除旧实例表并重命名新表。
DROP TABLE IF EXISTS algorithm_instances;
ALTER TABLE algorithm_instances_new RENAME TO algorithm_instances;

-- 4. 为任务实例关系和全局实例标识建立索引与唯一约束。
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_algorithm_unique ON algorithm_instances(task_id, algorithm_id);
CREATE INDEX IF NOT EXISTS idx_task_instances ON algorithm_instances(task_id);
CREATE INDEX IF NOT EXISTS idx_task_instances_camera ON algorithm_instances(camera_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_instances_instance_id ON algorithm_instances(instance_id);
