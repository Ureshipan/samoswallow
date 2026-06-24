-- Optional fixed external port for an app. When set, every instance of the app
-- publishes its primary container port on this host port, bound to 0.0.0.0 so
-- the service is reachable directly from outside (in addition to the Caddy
-- subdomain). When NULL, a random localhost port is assigned per instance and
-- the app is only reachable through Caddy (the original behaviour).
ALTER TABLE apps ADD COLUMN external_port INTEGER;
