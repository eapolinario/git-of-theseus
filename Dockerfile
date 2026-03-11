FROM nixos/nix:latest

# Enable flakes and disable sandbox (Nix sandbox is incompatible with Docker)
RUN echo "experimental-features = nix-command flakes" >> /etc/nix/nix.conf && \
    echo "sandbox = false" >> /etc/nix/nix.conf

WORKDIR /app

# Copy all project files
COPY . .

# Build the package using the nix flake
RUN nix build .#default --out-link /result

# Make the built binaries available on PATH
ENV PATH="/result/bin:${PATH}"

COPY container/run.sh /run.sh

CMD ["git-of-theseus-analyze", "--help"]
