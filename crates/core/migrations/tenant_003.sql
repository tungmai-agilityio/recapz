-- Which embedder produced each chunk vector. Vectors from different embedders are not comparable,
-- so search only compares a chunk with a query embedded by the same one.
ALTER TABLE chunks ADD COLUMN embedder TEXT NOT NULL DEFAULT 'hash-256';
