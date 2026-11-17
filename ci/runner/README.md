# Self-hosted build runner

Docker image + compose for a GitHub Actions self-hosted runner that builds the
Linux (AppImage, deb) and Android (APK, AAB) releases of BenNovel.

```bash
cp .env.example .env && chmod 600 .env   # put the runner token in ACCESS_TOKEN
docker compose up -d --build
```

Then set the repository variable `SELF_HOSTED_RUNNER=true`
(Settings → Secrets and variables → Actions → Variables) so
`.github/workflows/release.yml` sends its Linux and Android jobs here.
Pull requests from forks always build on GitHub's runners.

One runner, so jobs build one at a time; `gradle.properties` and
`CARGO_BUILD_JOBS` keep a build within 8 GB RAM + swap.

Keep `NDK_VERSION` here in step with the one in the workflow.
