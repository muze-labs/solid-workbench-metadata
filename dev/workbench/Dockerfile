# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim AS build
WORKDIR /src
COPY --from=workbench package.json package-lock.json ./
COPY --from=workbench apps/ apps/
COPY --from=workbench packages/ packages/
COPY --from=workbench broker/ broker/
COPY --from=workbench catalog/ catalog/
COPY --from=workbench local-packages/ local-packages/
COPY --from=workbench scripts/ scripts/
COPY --from=workbench design/ design/
RUN npm ci --no-audit --no-fund && npm run build

FROM nginx:alpine
COPY --from=build /src/www/ /usr/share/nginx/html/
COPY default.conf /etc/nginx/conf.d/default.conf
