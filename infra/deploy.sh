#!/bin/sh
set -ex

PROJECT_NAME=aelita-1374
CLUSTER_NAME=aelita-cluster
COMPUTE_ZONE=us-central1-f
VERSION=v$TRAVIS_BUILD_NUMBER

# Build Docker containers
cd static-binary
docker build -t=gcr.io/$PROJECT_NAME/aelita:$VERSION .
cd ../signup/
docker build -t=gcr.io/$PROJECT_NAME/signup:$VERSION .
cd ../infra/caddy/
docker build -t=gcr.io/$PROJECT_NAME/caddy:$VERSION .

# Install gcloud
cd ../infra/
wget https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/google-cloud-sdk-117.0.0-linux-x86_64.tar.gz
tar -xvf google-cloud-sdk-*
export PATH="`pwd`/google-cloud-sdk/bin/:$PATH"
gcloud --quiet components update
gcloud --quiet components update kubectl

# Log in to GCE
# GCLOUD_SERVICE_KEY was added by running:
#     key=`base64 -w0 service.json`
#     travis env --repo AelitaBot/aelita set GCLOUD_SERVICE_KEY $key
echo $GCLOUD_SERVICE_KEY | base64 --decode > $HOME/gcloud-service-key.json
gcloud --quiet auth activate-service-account \
  --key-file ${HOME}/gcloud-service-key.json
gcloud config set project $PROJECT_NAME
gcloud config set compute/zone $COMPUTE_ZONE
gcloud config set container/cluster $CLUSTER_NAME
gcloud --quiet container clusters get-credentials $CLUSTER_NAME

# Push to gcr.io
gcloud docker push gcr.io/$PROJECT_NAME/aelita:$VERSION

# Upgrade the pod
# PASSWORD envs were added by running:
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set POSTGRES_PIPELINES_PASSWORD $RAND
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set POSTGRES_CACHES_PASSWORD $RAND
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set POSTGRES_CONFIGS_PASSWORD $RAND
#     travis env --repo AelitaBot/aelita set GITHUB_PERSONAL_ACCESS_TOKEN yourmom
#     travis env --repo AelitaBot/aelita set GITHUB_WEHOOK_SECRET yourdad
#     travis env --repo AelitaBot/aelita set GITHUB_STATUS_WEHOOK_SECRET yoursis
#     travis env --repo AelitaBot/aelita set GITHUB_CLIENT_ID yourbro
#     travis env --repo AelitaBot/aelita set GITHUB_CLIENT_SECRET yourson
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set VIEW_SECRET $RAND
sed -i s:INSERT_CURRENT_VERION_HERE:${VERSION}:g aelita.yaml
sed -i s:INSERT_POSTGRES_PIPELINES_PASSWORD_HERE:${POSTGRES_PIPELINES_PASSWORD}:g aelita.yaml
sed -i s:INSERT_POSTGRES_CACHES_PASSWORD_HERE:${POSTGRES_CACHES_PASSWORD}:g aelita.yaml
sed -i s:INSERT_POSTGRES_CONFIGS_PASSWORD_HERE:${POSTGRES_CONFIGS_PASSWORD}:g aelita.yaml
sed -i s:INSERT_GITHUB_PERSONAL_ACCESS_TOKEN_HERE:${GITHUB_PERSONAL_ACCESS_TOKEN}:g aelita.yaml
sed -i s:INSERT_GITHUB_WEBHOOK_SECRET_HERE:${GITHUB_WEBHOOK_SECRET}:g aelita.yaml
sed -i s:INSERT_GITHUB_STATUS_WEBHOOK_SECRET_HERE:${GITHUB_STATUS_WEBHOOK_SECRET}:g aelita.yaml
sed -i s:INSERT_GITHUB_CLIENT_ID_HERE:${GITHUB_CLIENT_ID}:g aelita.yaml
sed -i s:INSERT_GITHUB_CLIENT_SECRET_HERE:${GITHUB_CLIENT_SECRET}:g aelita.yaml
sed -i s:INSERT_VIEW_SECRET_HERE:${VIEW_SECRET}:g aelita.yaml
kubectl apply -f aelita.yaml

