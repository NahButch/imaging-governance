# Space #1 — Docker SDK Space.
#
# `scripts/deploy_space.sh 01-dicom-qc` stages the workspace into `govcore_src/`
# (Cargo.toml + crates/govcore) next to this file before pushing, so the build
# context contains everything needed to compile the deterministic core.

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
# HF Spaces run as a non-root user with a writable home.
RUN useradd -m -u 1000 user
WORKDIR /home/user/app
COPY --from=build /wheels /wheels
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt /wheels/*.whl \
    && rm -rf /wheels
COPY --chown=user app.py ./
COPY --chown=user examples/ ./examples/
USER user
ENV GRADIO_SERVER_NAME=0.0.0.0 GRADIO_SERVER_PORT=7860
EXPOSE 7860
CMD ["python", "app.py"]
