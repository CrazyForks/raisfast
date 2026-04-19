-- 为 media 表添加图片宽高字段（非图片类型为 NULL）
ALTER TABLE media ADD COLUMN width INTEGER;
ALTER TABLE media ADD COLUMN height INTEGER;
