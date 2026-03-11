FROM python:3.12-slim

# Install uv
COPY --from=ghcr.io/astral-sh/uv:0.10.9 /uv /uvx /usr/local/bin/

# Install system dependencies (git is required by gitpython)
RUN apt-get update -q && \
    apt-get install -yqq --no-install-recommends git && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy project files
COPY pyproject.toml README.md LICENSE ./
COPY git_of_theseus/ ./git_of_theseus/
COPY container/run.sh /run.sh

# Install the package
RUN uv pip install --system --no-cache .

CMD ["git-of-theseus-analyze", "--help"]
