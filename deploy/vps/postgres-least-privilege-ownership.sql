\set ON_ERROR_STOP on
-- Run as the existing database administrator, on a verified backup copy first.
-- The migrator must own historical objects to apply future ALTER TABLE migrations.
BEGIN;
DO $$
DECLARE item record;
BEGIN
  FOR item IN
    SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public' AND c.relkind = 'r'
    ORDER BY c.relname
  LOOP
    EXECUTE format('ALTER TABLE public.%I OWNER TO deopt_migrator', item.relname);
  END LOOP;
  FOR item IN
    SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public' AND c.relkind = 'S'
    ORDER BY c.relname
  LOOP
    EXECUTE format('ALTER SEQUENCE public.%I OWNER TO deopt_migrator', item.relname);
  END LOOP;
  FOR item IN
    SELECT p.oid::regprocedure AS signature FROM pg_proc p
    JOIN pg_namespace n ON n.oid = p.pronamespace
    WHERE n.nspname = 'public'
  LOOP
    EXECUTE format('ALTER FUNCTION %s OWNER TO deopt_migrator', item.signature);
  END LOOP;
END $$;
ALTER SCHEMA public OWNER TO deopt_migrator;
COMMIT;
