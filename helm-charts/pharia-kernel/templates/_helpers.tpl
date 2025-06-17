{{/*
Expand the name of the chart.
*/}}
{{- define "pharia-kernel.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "pharia-kernel.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "pharia-kernel.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "pharia-kernel.labels" -}}
app: {{ include "pharia-kernel.fullname" . }}
helm.sh/chart: {{ include "pharia-kernel.chart" . }}
{{- if eq .Values.image.tag "latest" }}
helm.sh/updated: {{ now | date "20060102150405" | quote }}
{{- end }}
{{ include "pharia-kernel.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "pharia-kernel.selectorLabels" -}}
app.kubernetes.io/name: {{ include "pharia-kernel.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "pharia-kernel.serviceAccountName" -}}
{{- include "pharia-kernel.fullname" . }}
{{- end }}

{{/*
Generate environment variables for default namespaces
*/}}
{{- define "pharia-kernel.defaultNamespacesEnvVars" -}}
{{- if .Values.global.phariaAIConfigMap }}
{{- $phariaAIConfigMap := .Values.global.phariaAIConfigMap -}}
{{- $imagePullOpaqueSecretName := .Values.global.imagePullOpaqueSecretName -}}
{{- range .Values.defaultNamespaces }}
- name: NAMESPACES__{{ . | upper }}__CONFIG_URL
  valueFrom:
    configMapKeyRef:
      name: {{ $phariaAIConfigMap }}
      key: NAMESPACES__{{ . | upper }}__CONFIG_URL
- name: NAMESPACES__{{ . | upper }}__REGISTRY
  valueFrom:
    configMapKeyRef:
      name: {{ $phariaAIConfigMap }}
      key: NAMESPACES__{{ . | upper }}__REGISTRY
- name: NAMESPACES__{{ . | upper }}__BASE_REPOSITORY
  valueFrom:
    configMapKeyRef:
      name: {{ $phariaAIConfigMap }}
      key: NAMESPACES__{{ . | upper }}__BASE_REPOSITORY
{{- if $imagePullOpaqueSecretName }}
- name: NAMESPACES__{{ . | upper }}__REGISTRY_USER
  valueFrom:
    secretKeyRef:
      name: {{ $imagePullOpaqueSecretName }}
      key: registryUser
- name: NAMESPACES__{{ . | upper }}__REGISTRY_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ $imagePullOpaqueSecretName }}
      key: registryPassword
{{- end }}
{{- end }}
{{- end }}
{{- end }}
