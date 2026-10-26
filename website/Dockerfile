# Railway builds this with the service's root directory set to `website`.
# Stage 1 renders dist/ from src/ and the README on main (no npm dependencies needed).
FROM node:22-alpine AS build
WORKDIR /site
COPY build.mjs ./
COPY src ./src
COPY docs ./docs
COPY data ./data
COPY public ./public
COPY LEGAL.md ./
ARG GITHUB_TOKEN
RUN node build.mjs

# Stage 2 serves it with Caddy on $PORT.
FROM caddy:2-alpine
COPY Caddyfile /etc/caddy/Caddyfile
COPY --from=build /site/dist /srv
ENV PORT=8080
# The Content-Security-Policy allows the page's inline script and style by the hashes the build wrote.
CMD ["sh", "-c", "CSP_SCRIPT=\"$(cat /srv/.csp-script)\" CSP_STYLE=\"$(cat /srv/.csp-style)\" exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile"]
