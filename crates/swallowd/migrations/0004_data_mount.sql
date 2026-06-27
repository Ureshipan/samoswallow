-- Optional persistent storage for an app: a host directory bind-mounted into
-- every instance's container. Useful for databases or files that must survive
-- redeploys, reboots and container removal, and be reachable from the host
-- without entering the container.
--
-- `data_dir`   = absolute path on the host (created on first deploy if missing).
-- `mount_path` = where it appears inside the container; defaults to /data when
--                left empty. Both NULL => no mount (original behaviour).
ALTER TABLE apps ADD COLUMN data_dir TEXT;
ALTER TABLE apps ADD COLUMN mount_path TEXT;
