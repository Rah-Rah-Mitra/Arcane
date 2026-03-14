-- Add depth and page_count columns to sources table
ALTER TABLE sources ADD COLUMN depth INTEGER;
ALTER TABLE sources ADD COLUMN page_count INTEGER;
