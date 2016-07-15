#!/bin/sh
set -ex

PROJECT_NAME=aelita-1374
CLUSTER_NAME=aelita-cluster
COMPUTE_ZONE=us-central1-f
VERSION=v$TRAVIS_BUILD_NUMBER

# Build Docker container
cd static-binary
docker build -t=gcr.io/$PROJECT_NAME/aelita:$VERSION .

# Install gcloud
cd ../infra/
wget https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/google-cloud-sdk-117.0.0-linux-x86_64.tar.gz
tar -xvf google-cloud-sdk-*
./google-cloud-sdk/install.sh
export PATH="`pwd`/google-cloud-sdk/bin/:$PATH"
gcloud --quiet components update
gcloud --quiet components update kubectl

# Log in to GCE
# GCLOUD_SERVICE_KEY was added by running:
#     key=`base64 -w0 service.json`
#     travis env --repo AelitaBot/aelita set GCLOUD_SERVICE_KEY=$key
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
sed -i s:INSERT_CURRENT_VERION_HERE:${VERSION}:g aelita.yaml
kubectl apply -f aelita.yaml

