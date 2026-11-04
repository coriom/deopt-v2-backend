\set ON_ERROR_STOP on
BEGIN;
GRANT USAGE ON SCHEMA public TO deopt_runtime, deopt_backup;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM PUBLIC;
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM deopt_runtime;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM deopt_runtime;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO deopt_backup;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO deopt_backup;
GRANT SELECT ON _sqlx_migrations, wallet_read_challenges, wallet_read_sessions,
    wallet_read_rate, indexed_perp_trades, canonical_v2_perp_fees, canonical_v2_intent_proofs,
    canonical_v2_fill_provenance TO deopt_runtime;
GRANT INSERT, UPDATE, DELETE ON wallet_read_challenges, wallet_read_sessions, wallet_read_rate TO deopt_runtime;
REVOKE CREATE ON SCHEMA public FROM deopt_runtime, deopt_backup;
COMMIT;
ALTER DEFAULT PRIVILEGES FOR ROLE deopt_migrator IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
-- psql expands the current database name without embedding a deployment name.
SELECT format('REVOKE CONNECT, TEMPORARY ON DATABASE %I FROM PUBLIC', current_database()) \gexec
SELECT format('GRANT CONNECT ON DATABASE %I TO deopt_runtime, deopt_migrator, deopt_backup', current_database()) \gexec
