#!/bin/bash
# mev-ant database setup script
# Usage: ./deploy/setup-db.sh [password]
# If no password given, prompts interactively.

set -euo pipefail

DB_USER="mevant"
DB_NAME="mev_ant"

if [ "${1:-}" != "" ]; then
    PASSWORD="$1"
else
    read -rsp "Enter password for $DB_USER: " PASSWORD
    echo ""
    read -rsp "Confirm password: " CONFIRM
    echo ""
    if [ "$PASSWORD" != "$CONFIRM" ]; then
        echo "Passwords do not match."
        exit 1
    fi
fi

if [ -z "$PASSWORD" ]; then
    echo "Password cannot be empty."
    exit 1
fi

echo "Creating user $DB_USER..."
sudo -u postgres psql <<SQL
DO \$\$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '$DB_USER') THEN
        CREATE USER $DB_USER WITH PASSWORD '$PASSWORD';
    ELSE
        ALTER USER $DB_USER WITH PASSWORD '$PASSWORD';
    END IF;
END
\$\$;
SQL

echo "Creating database $DB_NAME..."
sudo -u postgres createdb -O "$DB_USER" "$DB_NAME" 2>/dev/null || echo "  Database already exists."

echo "Granting permissions..."
sudo -u postgres psql -d "$DB_NAME" <<SQL
GRANT ALL ON SCHEMA public TO $DB_USER;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO $DB_USER;
SQL

echo ""
echo "=== Done ==="
echo "Update serve.toml with:"
echo "  db_url = \"postgres://$DB_USER:<password>@localhost:5432/$DB_NAME\""
