 1. Create a GitHub account for the bot
 2. Get a Personal Access Token from GitHub, with "repo" permission
 3. Register an oAuth application
 2. Create a Google Cloud Platform (GCP) project
 3. Create a Google Container Engine (GKE) cluster (I use 3 micro servers)
 4. Create a GCP IAM Service Account (I call it "Aelita Deploy")
    with "Editor" role, and do furnish a private key as JSON;
    you don't need G Suite
 5. Enable Travis CI on a fork of AelitaBot/aelita, but do not build it yet
 6. Go into your fork and run these commands

        travis env set PROJECT_NAME <GCP project name>
        travis env set CLUSTER_NAME <GKE cluster name>
        travis env set COMPUTE_ZONE <GKE compute zone>
        travis env set POSTGRES_PIPELINES_PASSWORD <random>
        travis env set POSTGRES_CACHES_PASSWORD <random>
        travis env set POSTGRES_CONFIGS_PASSWORD <random>
        travis env set GITHUB_PERSONAL_ACCESS_TOKEN <given by GitHub>
        travis env set GITHUB_WEBHOOK_SECRET <randon>
        travis env set GITHUB_STATUS_WEBHOOK_SECRET <random>
        travis env set GITHUB_CLIENT_ID <given by GitHub>
        travis env set GITHUB_CLIENT_SECRET <given by GitHub>
        travis env set VIEW_SECRET <random>
        travis env set GCLOUD_SERVICE_KEY `base64 -w0 ../some-service.json`
        travis env set SIGNUP_DOMAIN <from DNS>
        travis env set BOT_DOMAIN <from DNS>
        travis env set BOT_USERNAME <from GitHub>

    You can also set a SENTRY_DSN, but that's optional.

 7. Edit aelita-ingress.yml,
    replacing `INSERT_BOT_DOMAIN` and `INSERT_SIGNUP_DOMAIN`
    with the applicable domains

 8. Run `kubectl apply -f aelita-ingress.yml`

 9. Create three GCE disks:
    postgres-caches, postgres-configs, postgres-pipelines

10. Run `kubectl apply -f aelita-volumes.yml`

11. Run `kubectl get ing` to see the entrypoint load balancer;
    copy the IP address into your DNS server, and wait for it to propagate

12. Push to your Travis-enabled branch,
    and wait for it to build and deploy

# Congratulations! You've set up a copy of hosted aelita!

