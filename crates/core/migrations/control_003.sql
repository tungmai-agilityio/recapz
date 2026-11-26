-- Per-workspace monthly cost cap in micro-USD, set by the server owner. NULL = the config default.
ALTER TABLE tenants ADD COLUMN monthly_cap_micro_usd INTEGER;
