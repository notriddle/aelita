#!/bin/bash
set -e

PROJECT_NAME=aelita-1374
CLUSTER_NAME=aelita-cluster
COMPUTE_ZONE=us-central1-f
CURRENT_VERSION=v$TRAVIS_BUILD_NUMBER

# Build Docker containers
cd static-binary
docker build -t=gcr.io/$PROJECT_NAME/aelita:$CURRENT_VERSION .
cd ../signup/
docker build -t=gcr.io/$PROJECT_NAME/signup:$CURRENT_VERSION .
cd ../infra/nginx/
cp -rv ../../signup/static/ .
docker build -t=gcr.io/$PROJECT_NAME/nginx:$CURRENT_VERSION .

# Install gcloud
cd ../
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
gcloud config set container/use_client_certificate true
gcloud --quiet container clusters get-credentials $CLUSTER_NAME

# Push to gcr.io
gcloud docker push gcr.io/$PROJECT_NAME/aelita:$CURRENT_VERSION
gcloud docker push gcr.io/$PROJECT_NAME/signup:$CURRENT_VERSION
gcloud docker push gcr.io/$PROJECT_NAME/nginx:$CURRENT_VERSION

# Upgrade the pod
# PASSWORD envs were added by running:
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set POSTGRES_PIPELINES_PASSWORD $RAND
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set POSTGRES_CACHES_PASSWORD $RAND
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set POSTGRES_CONFIGS_PASSWORD $RAND
#     travis env --repo AelitaBot/aelita set GITHUB_PERSONAL_ACCESS_TOKEN \
#                                            yourmom
#     travis env --repo AelitaBot/aelita set GITHUB_WEBHOOK_SECRET yourdad
#     travis env --repo AelitaBot/aelita set GITHUB_STATUS_WEBHOOK_SECRET \
#                                            yoursis
#     travis env --repo AelitaBot/aelita set GITHUB_CLIENT_ID yourbro
#     travis env --repo AelitaBot/aelita set GITHUB_CLIENT_SECRET yourson
#     RAND=`dd if=/dev/urandom of=/dev/stdout count=4096 | sha256sum -`
#     travis env --repo AelitaBot/aelita set VIEW_SECRET $RAND
for i in POSTGRES_PIPELINES_PASSWORD POSTGRES_CACHES_PASSWORD \
         POSTGRES_CONFIGS_PASSWORD GITHUB_PERSONAL_ACCESS_TOKEN \
         GITHUB_WEBHOOK_SECRET GITHUB_STATUS_WEBHOOK_SECRET \
         GITHUB_CLIENT_ID GITHUB_CLIENT_SECRET \
         VIEW_SECRET CURRENT_VERSION; do
    sed -i s:INSERT_${i}_HERE:${!i}:g aelita.yaml
done
kubectl apply -f aelita.yaml

