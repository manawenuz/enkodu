---
tags:
  - adr
  - mobile
  - companion
  - pwa
created: 2026-06-09
---

# ADR-001: Mobile Companion Path — PWA First

## Status

Superseded by [[05-Product/Missing Companion Clients PRD|Missing Companion Clients PRD]] on 2026-06-10.

Native Android and iOS companion apps are now in scope. A PWA may still be useful later as a status-only client, but it is no longer the primary mobile companion path.

## Context

Desktop companions (macOS, Linux, Windows) provide file submission, monitoring, and download for Enkodu. Mobile companions are missing (PRD §219-265).

Mobile platforms impose constraints that make native apps more complex:
- Android: Storage Access Framework for file picking, foreground service requirement for long transfers, background execution limits
- iOS: Files app integration, strict background constraints, App Store review policies

The PRD explicitly suggests considering "a mobile-friendly web companion or PWA before native Android, unless native file APIs are required" (§241) and "a web/PWA status client first, then native upload/download if the platform constraints are acceptable" (§264).

## Original Decision

**Adopt PWA-first approach for mobile companions.**

Build a Progressive Web App that:
- Uses existing queue API endpoints without modification
- Provides submit, monitor, download, reconcile flows via browser
- Works on both Android and iOS without platform-specific code
- Can be installed as a PWA on both platforms

Defer native Android/iOS apps until:
- Desktop companions are proven in production
- Clear user demand for native features (e.g., background uploads, system file picker integration) emerges
- Platform constraints (especially iOS background limits) are fully understood

## Consequences

### What gets easier
- Single codebase for both mobile platforms
- No platform-specific build pipelines or signing requirements
- No new API endpoints needed — all existing endpoints are HTTP-based and work from browsers
- Faster iteration and deployment (web release cycle)
- No App Store review delays
- Users can "install" to home screen on both Android and iOS

### What gets harder
- No direct filesystem access — relies on browser file picker (user must manually select files)
- No background execution — uploads/downloads pause when browser tab/PWA is closed
- No system-level notifications on iOS (PWA notifications are browser-limited)
- No automatic startup or persistent background service
- Large file uploads may be interrupted by browser limits

## API Compatibility Checklist

All existing queue API endpoints are compatible with PWA/browser:

| Endpoint | PWA Compatible | Notes |
|---|---|---|
| `POST /jobs/upload` | ✓ | Form-based upload works from browser |
| `GET /jobs/{id}` | ✓ | Standard HTTP GET |
| `GET /jobs/{id}/output` | ✓ | Download via browser |
| `GET /status` | ✓ | Standard HTTP GET |
| `GET /jobs/live` | ✓ | SSE or polling |
| `GET /control` | ✓ | Standard HTTP GET |
| `POST /control/{cmd}` | ✓ | Standard HTTP POST |
| `GET /settings` | ✓ | Standard HTTP GET |
| `POST /settings` | ✓ | Standard HTTP POST |
| `GET /jobs?status=done&limit=2000` | ✓ | Standard HTTP GET |

**No API changes required for PWA implementation.**

## Platform Constraints

### Android
- **File picking:** Browser file picker works; Storage Access Framework not accessible from PWA
- **Uploads:** Can run in foreground tab; Service Workers can cache but not run long background tasks
- **Downloads:** Browser download manager handles output files
- **Notifications:** Web Push API available; requires user permission
- **Installation:** PWA can be added to home screen
- **Storage:** Browser sandboxed storage; no direct filesystem access

### iOS
- **File picking:** Files app integration via browser file picker (user selects from Files)
- **Uploads:** Foreground only; iOS suspends background tabs aggressively
- **Downloads:** Saved to Files app via download attribute or user interaction
- **Notifications:** Web Push not fully supported; relies on in-app badges/alerts
- **Installation:** PWA can be added to home screen
- **Storage:** iOS Safari has strict storage quotas (~1GB); IndexedDB available

## Future Native Considerations

If native apps become necessary:
- Android: Use Storage Access Framework + WorkManager for background transfers
- iOS: Requires native Swift app with proper entitlements; background fetch limited to ~30 seconds
- Both: Would need API additions for push notifications, or use Firebase/APNS

## Superseding Decision

Build proper native Android and iOS companion apps.

Hard gate:

- The native app must check runtime AV1 hardware decode support before enabling video upgrade.
- Unsupported devices may view queue/status, but must not upload videos for AV1 conversion or download/save AV1 outputs locally.
- Android should use `MediaCodecList` / `MediaCodecInfo` hardware decode checks for AV1.
- iOS should use VideoToolbox `VTIsHardwareDecodeSupported` for AV1.

## Links

- PRD: [[Missing Companion Clients PRD]]
- Related code: `queue/` API endpoints, companion core logic
