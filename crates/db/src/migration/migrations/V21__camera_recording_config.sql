-- 按通道的录像配置（JSON 文本，NULL/空串 = 未配置，继承全局默认且默认关闭）
ALTER TABLE cameras ADD COLUMN recording_config TEXT NOT NULL DEFAULT '';
