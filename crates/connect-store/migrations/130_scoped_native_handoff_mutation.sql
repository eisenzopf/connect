-- Fresh-schema boundary: persistent exact scoped-handoff writer trigger
-- exceptions are installed from authority_writer_boundary::schema. This adds
-- no legacy mixed-version execution mode and no reusable IO grant.
PRAGMA user_version=130;
