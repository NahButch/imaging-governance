# Space #2 — Docker SDK Space.
# `scripts/deploy_space.sh 02-seg-governance` stages the workspace into
# `govcore_src/` before pushing so the build context can compile the core.

# ---- build the govcore Rust extension into a wheel ----
FROM rust:1-slim-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 python3-venv python3-pip \
    && rm -rf /var/lib/apt/lists/*
RUN python3 -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"
RUN pip install --no-cache-dir maturin
WORKDIR /build
COPY govcore_src/ ./govcore_src/
RUN maturin build --release -m govcore_src/crates/govcore/Cargo.toml -o /wheels

# ---- runtime image ----
FROM python:3.11-slim
RUN useradd -m -u 1000 user
WORKDIR /home/user/app
COPY --from=build /wheels /wheels
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt /wheels/*.whl \
    && rm -rf /wheels
COPY --chown=user app.py ./
COPY --chown=user model/ ./model/
USER user
ENV GRADIO_SERVER_NAME=0.0.0.0 GRADIO_SERVER_PORT=7860 MOCK_MODEL=1
EXPOSE 7860
CMD ["python", "app.py"]
