# Compose Service Lifecycle

Use this guide when a task needs a local Compose-managed service started or updated. Replace each placeholder with the values from the current checkout and Compose configuration.

## Start and verify

Select the Compose file, project, and service explicitly. Start the service and its declared dependencies with:

```bash
docker compose -p <project> -f <compose-file> up -d --build <service>
```

Compose builds the required image and reuses available build-cache layers. Verify the service and its health endpoint:

```bash
docker compose -p <project> -f <compose-file> ps <service>
curl -fsS <health-url>
```

## Iterate and apply changes

For web-only changes, use the repository's development server with HMR and proxy API requests to the running service. Find the command in the repository's scripts or development docs. This keeps each UI edit out of the container build loop.

When the task requires the running service to include completed changes, rebuild and recreate it once:

```bash
docker compose -p <project> -f <compose-file> up -d --build --no-deps <service>
```

Use `--no-deps` only when the service's dependencies are already running and unchanged. Otherwise omit it so Compose can start or update them. Keep web assets in a build stage separate from backend compilation so a web-only change leaves the backend layer cached.

## Preserve scope and data

Target only the selected Compose project and requested service, plus its required dependencies when they need to start or change. Preserve configured data volumes. Do not use `down -v` or remove volumes to apply an application update. Change a running service only when the task requests or authorizes that runtime update; otherwise report the build and verification results without deploying it.
