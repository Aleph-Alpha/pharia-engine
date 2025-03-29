{{/*
Helm template for rendering the content of the env.
*/}}
{{- define "pharia-kernel.envvar" -}}
- name: MEMORY_REQUEST
  valueFrom:
    resourceFieldRef:
      containerName: {{ include "pharia-common.names.fullname" . }}
      resource: requests.memory
- name: MEMORY_LIMIT
  valueFrom:
    resourceFieldRef:
      containerName: {{ include "pharia-common.names.fullname" . }}
      resource: limits.memory
- name: LOG_LEVEL
  value: {{ .Values.logLevel | quote }}
{{- if .Values.openTelemetryEndpoint }}
- name: OTEL_ENDPOINT
  value: {{ .Values.openTelemetryEndpoint | quote }}
{{- end }}
{{- if .Values.authorizationAddress }}
- name: AUTHORIZATION_URL
  value: {{ .Values.authorizationAddress | quote }}
{{- else if .Values.global.phariaAIConfigMap }}
- name: AUTHORIZATION_URL
  valueFrom:
    configMapKeyRef:
      name: {{ .Values.global.phariaAIConfigMap }}
      key: PHARIA_IAM_URL
{{- end }}
{{- if .Values.inferenceAddress }}
- name: INFERENCE_URL
  value: {{ .Values.inferenceAddress | quote }}
{{- else if .Values.global.phariaAIConfigMap }}
- name: INFERENCE_URL
  valueFrom:
    configMapKeyRef:
      name: {{ .Values.global.phariaAIConfigMap }}
      key: API_SCHEDULER_URL
{{- end }}
{{- if .Values.documentIndexAddress }}
- name: DOCUMENT_INDEX_URL
  value: {{ .Values.documentIndexAddress | quote }}
{{- else if .Values.global.phariaAIConfigMap }}
- name: DOCUMENT_INDEX_URL
  valueFrom:
    configMapKeyRef:
      name: {{ .Values.global.phariaAIConfigMap }}
      key: DOCUMENT_INDEX_URL
{{- end }}
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
{{- end -}}
