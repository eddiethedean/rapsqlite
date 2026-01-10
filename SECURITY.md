# Security Audit Report - rapsqlite

**Last Audit Date:** January 10, 2026

## Security Status

- ✅ **Clippy Security Lints**: Passed
- ✅ **Unsafe Code Blocks**: 0 found
- ❌ **Dependency Vulnerabilities**: 2 critical vulnerabilities, 1 medium vulnerability, 1 warning

## Identified Vulnerabilities

### Critical: pyo3 0.20.3

**Advisory ID:** RUSTSEC-2025-0020  
**Severity:** Critical  
**Issue:** Risk of buffer overflow in `PyString::from_object`  
**Current Version:** 0.20.3  
**Required Version:** >=0.24.1  
**URL:** https://rustsec.org/advisories/RUSTSEC-2025-0020

**Description:**
`PyString::from_object` took `&str` arguments and forwarded them directly to the Python C API without checking for terminating nul bytes. This could lead the Python interpreter to read beyond the end of the `&str` data and potentially leak contents of the out-of-bounds read (by raising a Python exception containing a copy of the data including the overflow).

**Impact:**
- Affects all packages using pyo3 0.20.x
- Fixed in pyo3 0.24.1+
- Requires code changes to upgrade (breaking changes between 0.20 and 0.24)

**Dependency Tree:**
```
pyo3 0.20.3
├── rapsqlite 0.0.1
└── pyo3-asyncio 0.20.0
    └── rapsqlite 0.0.1
```

**Recommended Action:**
Upgrade pyo3 and pyo3-asyncio to >=0.24.1 as part of Phase 1 roadmap improvements.

---

### Critical: sqlx 0.7.4

**Advisory ID:** RUSTSEC-2024-0363  
**Severity:** Critical  
**Issue:** Binary Protocol Misinterpretation caused by Truncating or Overflowing Casts  
**Current Version:** 0.7.4  
**Required Version:** >=0.8.1  
**URL:** https://rustsec.org/advisories/RUSTSEC-2024-0363

**Description:**
sqlx 0.7.x has vulnerabilities related to binary protocol misinterpretation caused by truncating or overflowing casts. This can lead to data corruption or security issues when processing database results.

**Impact:**
- Affects all versions of sqlx < 0.8.1
- Fixed in sqlx 0.8.1+
- Requires code changes to upgrade (breaking changes between 0.7 and 0.8)

**Dependency Tree:**
```
sqlx 0.7.4
└── rapsqlite 0.0.1
```

**Recommended Action:**
Upgrade sqlx to >=0.8.1 as part of Phase 1 roadmap improvements.

---

### Medium: rsa 0.9.10 (Indirect Dependency)

**Advisory ID:** RUSTSEC-2023-0071  
**Severity:** Medium (CVSS: 5.9)  
**Issue:** Marvin Attack: potential key recovery through timing sidechannels  
**Current Version:** 0.9.10  
**Status:** No fixed upgrade available  
**URL:** https://rustsec.org/advisories/RUSTSEC-2023-0071

**Description:**
The rsa crate has a vulnerability related to timing sidechannel attacks that could potentially allow key recovery.

**Impact:**
- This is an **indirect dependency** through sqlx-mysql
- **Not applicable** to rapsqlite builds using only the sqlite feature
- Only affects builds that include MySQL support (not used by rapsqlite)

**Dependency Path (if MySQL features enabled):**
```
rsa 0.9.10
└── sqlx-mysql 0.7.4
    ├── sqlx-macros-core 0.7.4
    │   └── sqlx-macros 0.7.4
    │       └── sqlx 0.7.4
    │           └── rapsqlite 0.0.1
    └── sqlx 0.7.4
```

**Recommended Action:**
- This vulnerability does NOT affect rapsqlite as we only use the sqlite feature
- Upgrading sqlx to 0.8.1+ will resolve this indirect dependency issue
- No immediate action required (false positive for sqlite-only builds)

---

### Warning: paste 1.0.15 (Unmaintained)

**Status:** Unmaintained  
**Date:** October 7, 2024  
**Current Version:** 1.0.15

**Description:**
The paste crate is marked as unmaintained. While this doesn't indicate a current security vulnerability, unmaintained packages may not receive security updates in the future.

**Impact:**
- Low immediate risk
- Indirect dependency through sqlx macros
- Should be monitored for alternatives or updates

**Recommended Action:**
- Monitor for updates or alternatives
- Will be addressed when upgrading sqlx to 0.8.1+
- Low priority

## Security Practices

### Code Security
- ✅ No unsafe code blocks in codebase
- ✅ All code passes clippy security-focused lints
- ✅ Uses safe Rust APIs exclusively
- ✅ Only uses sqlite feature of sqlx (minimizes attack surface)

### Dependency Management
- 🔄 Regular security audits recommended via `cargo audit`
- 🔄 Monitor for dependency updates
- 🔄 Update dependencies as part of regular maintenance
- ✅ Using minimal feature set (sqlite only, not mysql/postgres)

## Running Security Checks

### Cargo Audit
```bash
cargo install cargo-audit
cargo audit
```

### Clippy Security Lints
```bash
cargo clippy --lib --all-features -- -W clippy::suspicious -W clippy::correctness
```

### Check for Unsafe Code
```bash
grep -r "unsafe {" src/ --include="*.rs"
```

## Update Schedule

Security audits should be run:
- Before each release
- Weekly via automated CI/CD (see `.github/workflows/security.yml`)
- After any dependency updates

## Priority Actions

1. **Priority 1:** Upgrade pyo3 from 0.20.3 to >=0.24.1 (Critical)
2. **Priority 2:** Upgrade sqlx from 0.7.4 to >=0.8.1 (Critical)
3. **Priority 3:** Monitor paste crate for updates/alternatives (Low)

## Reporting Security Issues

If you discover a security vulnerability, please email: odosmatthews@gmail.com

Do not open public GitHub issues for security vulnerabilities.

