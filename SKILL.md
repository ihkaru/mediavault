---
name: mediavault
description: Universal Media & Document Vault API (Pure-Rust). Resumable TUS chunk uploads, direct multipart ingestion, SIMD-accelerated WebP thumbnailing, PDF text extraction, sub-50ms Meilisearch retrieval, and Coolify cloud-native deployment.
---

# 🦀 MediaVault Skill & Complete API Reference Manual

MediaVault is an ultra-fast, memory-efficient, 100% pure-Rust media and document ingestion platform. It decouples media upload, background processing, cryptographic verification, and instant search retrieval into a unified microservice designed for multi-tenant applications (SaaS, microservices, bots, mobile, and web frontends).

```
                    ┌────────────────────────────────────────────────────────┐
                    │               Client Applications / APIs               │
                    └───────────┬────────────────────────────────┬───────────┘
                                │ 1. Direct / Resumable Ingestion │ 6. Instant Search (<50ms)
                                ▼                                ▼
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│                                  MediaVault Core (Rust / Axum)                              │
│                                                                                            │
│  ┌───────────────────────┐  ┌──────────────────────┐  ┌─────────────────────────────────┐  │
│  │   Auth & Rate Guard   │  │   MediaService       │  │       Search Dispatcher         │  │
│  │  (X-API-Key / Bearer) │  │  (Orchestration Core)│  │   (Meilisearch + SQL Fallback)  │  │
│  └───────────────────────┘  └──────────┬───────────┘  └────────────────┬────────────────┘  │
│                                        │                               │                   │
│         ┌──────────────────────────────┼──────────────────────────────┐│                   │
│         ▼                              ▼                              ▼▼                   │
│  ┌──────────────┐              ┌──────────────┐              ┌────────────────┐            │
│  │ Image Worker │              │  PDF Worker  │              │ Storage Driver │            │
│  │  (image-rs)  │              │   (lopdf)    │              │ (Local / S3)   │            │
│  └──────────────┘              └──────────────┘              └────────────────┘            │
└────────────────────────────────────────┬───────────────────────────────┬───────────────────┘
                                         ▼                               ▼
                      ┌─────────────────────────────────┐ ┌──────────────────────────────────┐
                      │    Storage (Garage S3 / FS)     │ │    Search Engine (Meilisearch)   │
                      └─────────────────────────────────┘ └──────────────────────────────────┘
```

---

## 1. Architectural Principles & SOLID Design

MediaVault is built strictly adhering to the **SOLID** architectural design principles:

1. **Single Responsibility Principle (SRP)**:
   - `src/api/`: Handles exclusively HTTP transport concerns, header extraction, and JSON response formatting.
   - `src/services/media.rs`: Encapsulates business logic, file hashing, coordination of background tasks, and session lifecycle.
   - `src/storage/`: Focuses solely on byte-level persistence (putting, reading, checking existence, deleting).
   - `src/processor/`: Isolated media manipulation (WebP encoding, PDF token parsing).
   - `src/search/meili.rs`: Manages HTTP communication and schema configuration with the search index.

2. **Open/Closed Principle (OCP)**:
   - Media processors are modularly registered in `analyze_and_process`. Additional file types (e.g., audio, video transcoding, vector embeddings) can be plugged in without modifying storage or routing layers.
   - The storage layer implements the `StorageBackend` trait; new cloud providers (e.g., Azure Blob, Google Cloud Storage, IPFS) can be introduced without changing service consumers.

3. **Liskov Substitution Principle (LSP)**:
   - Both `LocalStorage` and `S3Storage` implement `StorageBackend`. The core service operates identically regardless of whether files are written to NVMe disks or an S3-compatible Garage cluster.

4. **Interface Segregation Principle (ISP)**:
   - Traits and contracts are narrowly tailored. `StorageBackend` requires only the essential I/O functions (`put_object`, `get_object`, `delete_object`, `exists`), preventing bloated god-interfaces.

5. **Dependency Inversion Principle (DIP)**:
   - High-level application state (`AppState` and `MediaService`) depends on the abstraction `DynStorage` (`Arc<dyn StorageBackend>`), injected during server bootstrapping.

---

## 2. Authentication & Security Specifications

### 2.1 API Key Headers
When configured with an `API_KEY` in environment variables, all `/api/v1/*` routes require authentication. The key can be passed via either:

```http
X-API-Key: <your_secret_api_key>
```
*or*
```http
Authorization: Bearer <your_secret_api_key>
```

If `API_KEY` is omitted or empty in `.env`, the service operates in open mode (useful for internal Docker networks or local sandbox development).

### 2.2 Multi-Tenant Header (`X-App-ID`)
To partition assets between multiple applications, clients should provide the `X-App-ID` header:

```http
X-App-ID: billing_service
```

If not provided, the asset is tagged under the default identifier `"general"`.

---

## 3. Comprehensive REST API Contract

Base URL: `https://<vault_domain>` or `http://localhost:8080`

### 3.1 Health & Diagnostics

#### `GET /healthz`
Returns system health, connected database status, active storage driver, and search engine reachability.

- **Auth Required**: No
- **Response `200 OK`**:
```json
{
  "status": "healthy",
  "service": "mediavault",
  "version": "0.1.0",
  "storage_backend": "local",
  "database": "connected",
  "meilisearch": {
    "enabled": true,
    "status": "connected"
  }
}
```

#### `GET /metrics`
Returns real-time aggregate statistics.

- **Auth Required**: No
- **Response `200 OK`**:
```json
{
  "total_files": 1420,
  "total_bytes": 1085429104,
  "storage_engine": "local",
  "meili_enabled": true
}
```

---

### 3.2 Direct Multipart File Upload

#### `POST /api/v1/files/upload`
Uploads a single file immediately. Handles MIME type deduction, SHA-256 fingerprinting, WebP thumbnail generation (for images), and PDF text extraction.

- **Auth Required**: Yes (`X-API-Key` or Bearer)
- **Headers**:
  - `Content-Type: multipart/form-data`
  - `X-App-ID: <app_identifier>` (Optional)
- **Multipart Form Fields**:
  - `file` (File, Required): The binary payload.
  - `app_id` (Text, Optional): Overrides `X-App-ID` header.
  - `tags` (Text, Optional): Comma-separated tags (e.g. `invoice,finance,q3`).
  - `metadata` (Text, Optional): Arbitrary JSON string for custom client metadata.

- **Response `201 Created`**:
```json
{
  "success": true,
  "data": {
    "id": "e0b462da-4b13-41c1-8406-8d626620583a",
    "app_id": "billing_service",
    "filename": "invoice_q3_2026.pdf",
    "mime_type": "application/pdf",
    "file_size": 184520,
    "sha256": "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4",
    "has_thumbnail": false,
    "download_url": "/api/v1/files/e0b462da-4b13-41c1-8406-8d626620583a/raw",
    "thumbnail_url": null,
    "tags": ["invoice", "finance", "q3"],
    "created_at": "2026-10-08T03:00:00Z"
  }
}
```

- **Error Codes**:
  - `400 BAD_REQUEST`: Empty payload or file exceeds `MAX_UPLOAD_SIZE_MB`.
  - `401 UNAUTHORIZED`: Missing or invalid API key.
  - `500 STORAGE_ERROR`: Underlying storage driver write failure.

---

### 3.3 TUS-Compatible Resumable Upload Protocol

Designed for large files (100MB+, videos, raw dumps, high-resolution scans) where network interruptions are likely.

#### Step 1: Initialize Resumable Session
`POST /api/v1/files/resumable`

- **Auth Required**: Yes
- **Headers**:
  - `Content-Type: application/json`
  - `X-App-ID: <app_identifier>` (Optional)
- **Request Body**:
```json
{
  "filename": "annual_financial_audit.pdf",
  "total_size": 41943040,
  "mime_type": "application/pdf",
  "app_id": "finance_app",
  "tags": ["audit", "annual", "2026"]
}
```

- **Response `201 Created`**:
- **Headers**:
  - `Location: /api/v1/files/resumable/3d9b010b-851a-4c07-9259-71512db47ee1`
  - `Upload-Offset: 0`
- **Body**:
```json
{
  "success": true,
  "data": {
    "id": "3d9b010b-851a-4c07-9259-71512db47ee1",
    "filename": "annual_financial_audit.pdf",
    "total_size": 41943040,
    "current_offset": 0,
    "upload_url": "/api/v1/files/resumable/3d9b010b-851a-4c07-9259-71512db47ee1",
    "expires_at": "2026-10-09T03:00:00Z"
  }
}
```

#### Step 2: Inquire Upload Offset (HEAD)
`HEAD /api/v1/files/resumable/{session_id}`

Allows clients to resume an interrupted upload by asking the server how many bytes have been successfully stored.

- **Auth Required**: Yes
- **Response `200 OK`**:
  - `Upload-Offset: 15728640` (Server has 15 MB)
  - `Upload-Length: 41943040` (Total file size)
  - Content length: `0`

#### Step 3: Stream Chunk Bytes (PATCH)
`PATCH /api/v1/files/resumable/{session_id}`

Appends chunk bytes to the temporary assembly file.

- **Auth Required**: Yes
- **Headers**:
  - `Content-Type: application/offset+octet-stream`
  - `Upload-Offset: 15728640` (Must match current server offset)
- **Body**: Binary chunk bytes.

- **Intermediate Chunk Response (`204 No Content`)**:
  - `Upload-Offset: 26214400` (Updated offset)

- **Final Chunk Response (`201 Created`)**:
  Triggered automatically when `current_offset == total_size`. The server finalizes the file, executes media analysis, and writes the permanent database record.
```json
{
  "success": true,
  "status": "completed",
  "data": {
    "id": "a9841804-0ee2-4e94-88aa-38e55e094248",
    "app_id": "finance_app",
    "filename": "annual_financial_audit.pdf",
    "mime_type": "application/pdf",
    "file_size": 41943040,
    "sha256": "3cb0179f8e434346648f...",
    "has_thumbnail": false,
    "download_url": "/api/v1/files/a9841804-0ee2-4e94-88aa-38e55e094248/raw",
    "thumbnail_url": null,
    "tags": ["audit", "annual", "2026"],
    "created_at": "2026-10-08T03:05:00Z"
  }
}
```

---

### 3.4 File Retrieval, Streaming & Deletion

#### `GET /api/v1/files/{id}`
Returns complete metadata for a specific file.

- **Auth Required**: Yes
- **Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "id": "e0b462da-4b13-41c1-8406-8d626620583a",
    "app_id": "billing_service",
    "filename": "invoice_q3_2026.pdf",
    "mime_type": "application/pdf",
    "file_size": 184520,
    "sha256": "8f434346648f6b96...",
    "has_thumbnail": false,
    "download_url": "/api/v1/files/e0b462da-4b13-41c1-8406-8d626620583a/raw",
    "thumbnail_url": null,
    "tags": ["invoice", "finance"],
    "created_at": "2026-10-08T03:00:00Z"
  }
}
```

#### `GET /api/v1/files/{id}/raw`
Streams the raw original file bytes with appropriate `Content-Type` and `Content-Disposition: inline; filename="..."`.

- **Auth Required**: Yes
- **Response Headers**:
  - `Content-Type: <mime_type>`
  - `Content-Disposition: inline; filename="<original_name>"`
  - `Content-Length: <bytes>`

#### `GET /api/v1/files/{id}/thumbnail`
Streams the generated high-quality WebP thumbnail.

- **Auth Required**: Yes
- **Response Headers**:
  - `Content-Type: image/webp`
  - `Cache-Control: public, max-age=31536000, immutable`

#### `DELETE /api/v1/files/{id}`
Performs a cascade deletion:
1. Deletes raw object from storage driver.
2. Deletes thumbnail object (if present).
3. Deletes record from database.
4. Removes document from Meilisearch index.

- **Auth Required**: Yes
- **Response `200 OK`**:
```json
{
  "success": true,
  "message": "File e0b462da-4b13-41c1-8406-8d626620583a deleted successfully"
}
```

#### `GET /api/v1/files`
List files with pagination and metadata filtering.

- **Auth Required**: Yes
- **Query Parameters**:
  - `app_id` (string, optional): Filter by application ID.
  - `mime_type` (string, optional): Exact MIME or prefix wildcard (e.g. `image/*`, `application/pdf`).
  - `limit` (integer, optional, default: 20, max: 100).
  - `offset` (integer, optional, default: 0).
- **Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "items": [...],
    "count": 20,
    "limit": 20,
    "offset": 0
  }
}
```

---

### 3.5 Sub-50ms Instant Search Retrieval

#### `GET /api/v1/search`
Queries assets using instant, typo-tolerant full-text search. Searches across:
- `filename`
- `extracted_text` (extracted PDF document body)
- `tags`
- `metadata_json`

- **Auth Required**: Yes
- **Query Parameters**:
  - `q` (string, optional): Search keyword.
  - `app_id` (string, optional): Scope search to specific application.
  - `mime_type` (string, optional): Filter by MIME type (e.g. `image/*`, `application/pdf`).
  - `limit` (integer, optional, default: 20).
  - `offset` (integer, optional, default: 0).

- **Response `200 OK`**:
```json
{
  "success": true,
  "engine": "meilisearch",
  "data": {
    "hits": [
      {
        "id": "e0b462da-4b13-41c1-8406-8d626620583a",
        "app_id": "billing_service",
        "filename": "invoice_q3_2026.pdf",
        "mime_type": "application/pdf",
        "file_size": 184520,
        "sha256": "8f434346648f...",
        "has_thumbnail": false,
        "thumbnail_url": null,
        "download_url": "/api/v1/files/e0b462da-4b13-41c1-8406-8d626620583a/raw",
        "tags": ["invoice", "finance", "q3"],
        "snippet": "...Invoice Statement #INV-2026-09 for Q3 Cloud Services total due...",
        "created_at": "2026-10-08T03:00:00Z"
      }
    ],
    "estimated_total_hits": 1,
    "processing_time_ms": 3,
    "query": "invoice",
    "limit": 20,
    "offset": 0
  }
}
```

> **Automatic Failover Behavior**: If Meilisearch is temporarily offline, restarting, or disabled, `engine` will automatically indicate `"database_fallback"`. The request will execute against SQLite SQL LIKE queries, returning identical schema structures without throwing HTTP errors.

---

## 4. Client Integration Examples

### 4.1 cURL CLI
```bash
# Upload a PDF
curl -X POST "https://vault.yourdomain.com/api/v1/files/upload" \
  -H "X-API-Key: mv_secret_prod_key" \
  -H "X-App-ID: document_manager" \
  -F "file=@/home/user/contracts/agreement.pdf" \
  -F "tags=legal,agreement,2026"

# Search for the document
curl -X GET "https://vault.yourdomain.com/api/v1/search?q=agreement&app_id=document_manager" \
  -H "X-API-Key: mv_secret_prod_key"
```

### 4.2 TypeScript / Bun / Node.js
```typescript
import { readFileSync } from "fs";

const VAULT_URL = "https://vault.yourdomain.com";
const API_KEY = process.env.MEDIAVAULT_API_KEY!;

// 1. Direct Upload
async function uploadMedia(filePath: string, filename: string, tags: string[]) {
  const fileBytes = readFileSync(filePath);
  const formData = new FormData();
  formData.append("file", new Blob([fileBytes]), filename);
  formData.append("tags", tags.join(","));

  const res = await fetch(`${VAULT_URL}/api/v1/files/upload`, {
    method: "POST",
    headers: {
      "X-API-Key": API_KEY,
      "X-App-ID": "backend_service",
    },
    body: formData,
  });

  return await res.json();
}

// 2. Instant Search
async function searchMedia(query: string) {
  const url = new URL(`${VAULT_URL}/api/v1/search`);
  url.searchParams.set("q", query);

  const res = await fetch(url.toString(), {
    headers: { "X-API-Key": API_KEY },
  });

  return await res.json();
}
```

### 4.3 Python 3.10+
```python
import requests

VAULT_URL = "https://vault.yourdomain.com"
API_KEY = "mv_secret_prod_key"
HEADERS = {"X-API-Key": API_KEY, "X-App-ID": "python_worker"}

# Direct Upload
with open("dataset_report.pdf", "rb") as f:
    files = {"file": ("dataset_report.pdf", f, "application/pdf")}
    data = {"tags": "analytics,dataset,q3"}
    resp = requests.post(f"{VAULT_URL}/api/v1/files/upload", headers=HEADERS, files=files, data=data)
    file_info = resp.json()["data"]
    print("Uploaded File ID:", file_info["id"])

# Search
search_resp = requests.get(
    f"{VAULT_URL}/api/v1/search",
    headers=HEADERS,
    params={"q": "dataset", "mime_type": "application/pdf"}
)
print("Search Hits:", search_resp.json()["data"]["hits"])
```

### 4.4 PHP 8.2+ / Laravel 11
```php
use Illuminate\Support\Facades\Http;

$vaultUrl = config('services.mediavault.url');
$apiKey   = config('services.mediavault.key');

// Direct Upload
$response = Http::withHeaders([
    'X-API-Key' => $apiKey,
    'X-App-ID'  => 'laravel_portal',
])->attach(
    'file', file_get_contents($uploadedFile->getRealPath()), $uploadedFile->getClientOriginalName()
)->post("{$vaultUrl}/api/v1/files/upload", [
    'tags' => 'billing,statement',
]);

$fileData = $response->json()['data'];
$fileId   = $fileData['id'];
$rawUrl   = $fileData['download_url'];
```

---

## 5. Coolify Deployment & Production Checklist

### 5.1 Environment Variables Matrix

| Variable | Default Value | Description |
| :--- | :--- | :--- |
| `PORT` | `8080` | Internal HTTP port. |
| `API_KEY` | *(empty)* | Master API key required for incoming requests. |
| `DATABASE_URL` | `sqlite:/app/data/mediavault.db?mode=rwc` | SQLite database URI with auto-migrations. |
| `STORAGE_BACKEND` | `local` | Storage driver: `local` or `s3`. |
| `STORAGE_LOCAL_DIR`| `/app/data/storage` | Local directory when `STORAGE_BACKEND=local`. |
| `S3_ENDPOINT` | `http://garage:3900` | S3 endpoint (Garage S3, Cloudflare R2, MinIO). |
| `S3_BUCKET` | `mediavault` | Destination S3 bucket. |
| `MEILI_ENABLED` | `true` | Enables/disables Meilisearch indexing. |
| `MEILI_HOST` | `http://meilisearch:7700` | Meilisearch service endpoint. |
| `MEILI_MASTER_KEY` | *(empty)* | Master authentication key for Meilisearch. |
| `MAX_UPLOAD_SIZE_MB`| `250` | Maximum allowed upload payload size in MB. |
| `THUMBNAIL_MAX_WIDTH`| `400` | Maximum width of generated WebP thumbnail. |
| `THUMBNAIL_MAX_HEIGHT`| `400` | Maximum height of generated WebP thumbnail. |

### 5.2 Coolify Deployment Steps
1. Create a new **Docker Compose** resource in Coolify.
2. Select GitHub repository: `ihkaru/mediavault`, branch `main`.
3. Fill in the Environment Variables as listed above.
4. Set your custom FQDN (e.g. `https://vault.yourdomain.com`).
5. Enable **Auto Deploy (Webhook / GitHub App)**.
6. Click **Deploy**. The `cargo-chef` cache builds subsequent commits in ~12 seconds.
