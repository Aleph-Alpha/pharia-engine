#!/usr/bin/env bash

set -e

git clone --depth 1 https://token:$HELM_UPDATE_IMAGE_TAG_TOKEN@gitlab.aleph-alpha.de/product/shared-helm-charts.git helm
pushd helm
yq -iP ".deployment.image.tag = \"$NEW_IMAGE_TAG\""  pharia-kernel/pharia-kernel/values.yaml
cat pharia-kernel/pharia-kernel/values.yaml
git config user.email "$GITLAB_USER_EMAIL"
git config user.name "$GITLAB_USER_NAME"
git commit -am "Update image tag to $NEW_IMAGE_TAG"
# TODO: bump patch version of chart
# This is not possible until https://gitlab.com/gitlab-org/gitlab/-/issues/389060 lands in our gitlab instance
# - git push $CI_REPOSITORY_URL main
git push https://token:$HELM_UPDATE_IMAGE_TAG_TOKEN@gitlab.aleph-alpha.de/product/shared-helm-charts.git main
popd
