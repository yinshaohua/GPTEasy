CREATE TABLE state_metadata (
    singleton_id       INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    database_uuid      TEXT NOT NULL UNIQUE
                       CHECK (length(database_uuid) = 36),
    schema_fingerprint TEXT NOT NULL
                       CHECK (
                           length(schema_fingerprint) = 64
                           AND schema_fingerprint NOT GLOB '*[^0-9a-f]*'
                       ),
    created_at          TEXT NOT NULL CHECK (length(trim(created_at)) > 0)
) STRICT;

CREATE TABLE schema_migrations (
    version    INTEGER PRIMARY KEY CHECK (version > 0),
    name       TEXT NOT NULL UNIQUE CHECK (length(trim(name)) > 0),
    checksum   TEXT NOT NULL
               CHECK (
                   length(checksum) = 64
                   AND checksum NOT GLOB '*[^0-9a-f]*'
               ),
    applied_at TEXT NOT NULL CHECK (length(trim(applied_at)) > 0)
) STRICT;

CREATE TABLE providers (
    id            TEXT PRIMARY KEY CHECK (length(trim(id)) > 0),
    provider_kind TEXT NOT NULL
                  CHECK (provider_kind IN ('built_in_recommended', 'custom')),
    built_in_key  TEXT UNIQUE,
    display_name  TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    base_url      TEXT,
    api_key       TEXT,
    default_model TEXT,
    created_at    TEXT NOT NULL CHECK (length(trim(created_at)) > 0),
    updated_at    TEXT NOT NULL CHECK (length(trim(updated_at)) > 0),
    CHECK (
        (
            provider_kind = 'built_in_recommended'
            AND built_in_key IS NOT NULL
            AND length(trim(built_in_key)) > 0
        )
        OR (provider_kind = 'custom' AND built_in_key IS NULL)
    )
) STRICT;

CREATE TABLE provider_verifications (
    provider_id             TEXT PRIMARY KEY
                            REFERENCES providers(id) ON DELETE CASCADE,
    combination_fingerprint TEXT NOT NULL
                            CHECK (
                                length(combination_fingerprint) = 64
                                AND combination_fingerprint NOT GLOB '*[^0-9a-f]*'
                            ),
    verified_at             TEXT NOT NULL CHECK (length(trim(verified_at)) > 0),
    contract_version        TEXT NOT NULL CHECK (length(trim(contract_version)) > 0)
) STRICT;

CREATE TABLE managed_environments (
    id                  TEXT PRIMARY KEY CHECK (length(trim(id)) > 0),
    environment_kind    TEXT NOT NULL
                        CHECK (environment_kind IN ('native_codex', 'wsl2')),
    platform_identity   TEXT NOT NULL CHECK (length(trim(platform_identity)) > 0),
    display_name        TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    current_provider_id TEXT
                        REFERENCES providers(id) ON DELETE RESTRICT,
    first_seen_at       TEXT NOT NULL CHECK (length(trim(first_seen_at)) > 0),
    last_seen_at        TEXT NOT NULL CHECK (length(trim(last_seen_at)) > 0),
    UNIQUE (environment_kind, platform_identity)
) STRICT;

CREATE TABLE app_settings (
    singleton_id              INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    locale                    TEXT NOT NULL
                              CHECK (locale IN ('system', 'zh-CN', 'en-US')),
    theme                     TEXT NOT NULL
                              CHECK (theme IN ('system', 'light', 'dark')),
    launch_at_login_desired   INTEGER NOT NULL
                              CHECK (launch_at_login_desired IN (0, 1)),
    close_to_tray_notice_seen INTEGER NOT NULL
                              CHECK (close_to_tray_notice_seen IN (0, 1)),
    onboarding_completed      INTEGER NOT NULL
                              CHECK (onboarding_completed IN (0, 1)),
    last_update_check_at      TEXT,
    updated_at                TEXT NOT NULL CHECK (length(trim(updated_at)) > 0)
) STRICT;
