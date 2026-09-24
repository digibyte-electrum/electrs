![Logo](logo/logo-digibyte.svg)

# Electrs for DigiByte

This repository is a DigiByte adaptation of [electrs](https://github.com/romanz/electrs),
the efficient Rust Electrum server originally written for Bitcoin. It keeps the
upstream project structure and protocol implementation, while adding DigiByte
network support and deployment examples for TrueNAS Apps.

An electrs server indexes the blockchain supplied by a local DigiByte full node.
The resulting index lets an Electrum-compatible DigiByte wallet query balances,
history, addresses, and transactions without downloading the entire chain into
the wallet. The full node remains the source of consensus data; electrs is the
indexing and Electrum-protocol service.

This project is intended for personal or small-scale use. Do not expose the
Electrum service or the DigiByte RPC interface directly to the public Internet
without appropriate access control, firewall rules, and operational hardening.

## DigiByte and TrueNAS architecture

The recommended deployment contains two applications on one private Docker
bridge network:

```text
Electrum DigiByte wallet
          |
          | TCP 50011 (Electrum protocol)
          v
     dgb_electrs  ---- RPC 14022 / P2P 12024 ---->  dgb_full_node
       index                                              |
                                                        blockchain
```

The DigiByte node must be fully synchronized before electrs can build a useful
index. It must also run with `txindex=1`, because electrs needs transaction
lookup by transaction ID.

The containers use the shared network `dgb-electrum-net`. Inside that network,
electrs reaches the node at `digibyte:14022` and `digibyte:12024`; these names
are container-network addresses and do not depend on the TrueNAS host IP.

## Why electrs uses port 50011

The electrs process listens on port `50001` inside its container, which is the
usual plaintext Electrum protocol port. TrueNAS publishes it as host port
`50011`:

```text
TrueNAS host 50011 -> electrs container 50001
```

This gives DigiByte a stable, recognizable port while avoiding a collision with
other Electrum servers, such as Bitcoin electrs instances that commonly publish
host port `50001`. DigiByte wallets should therefore connect to
`<truenas-host>:50011` using plaintext (`:t` in Electrum server notation).

Port `50011` is only a host-side mapping; it does not change the Electrum
protocol port used between the containers and does not alter DigiByte RPC port
`14022` or P2P port `12024`.

## Prerequisites

- A TrueNAS SCALE installation with Docker/Compose-based Custom Apps support.
- A storage dataset for the DigiByte node data.
- A separate dataset for the electrs RocksDB index and logs.
- Enough storage for the DigiByte blockchain and the electrs index.
- A synchronized DigiByte node image, such as `blocknetdx/digibyte:latest`.
- The DigiByte electrs image published by this project:
  `ghcr.io/digibyte-electrum/electrs:digibyte-support`.

Use dataset paths appropriate for your system. The examples intentionally use
generic paths and credentials; do not copy production passwords from examples.

## Step 1: Create the DigiByte node Custom App

In TrueNAS, open **Apps**, choose **Discover Apps** or **Launch Docker Image**
(the wording depends on the TrueNAS release), select the Custom App/YAML
workflow, and create the node first. Paste the following Compose-compatible
custom YAML:

```yaml
networks:
  dgb-electrum-net:
    driver: bridge
    name: dgb-electrum-net

services:
  digibyte:
    command: >-
      digibyted -server=1 -listen=1 -txindex=1 -dbcache=256 -maxmempool=64
      -par=0 -rpcuser=dgb_electrum
      -rpcpassword=CHANGE_THIS_TO_A_LONG_RANDOM_PASSWORD
      -rpcbind=0.0.0.0 -rpcallowip=127.0.0.1/32
      -rpcallowip=192.168.1.0/24 -rpcallowip=172.16.0.0/12
      -rpcport=14022 -maxconnections=32
    container_name: dgb_full_node
    deploy:
      resources:
        limits:
          cpus: '6.0'
          memory: 8G
        reservations:
          memory: 4096M
    image: blocknetdx/digibyte:latest
    networks:
      dgb-electrum-net:
        aliases:
          - digibyte
    ports:
      - '14022:14022'
      - '12024:12024'
    restart: unless-stopped
    volumes:
      - /mnt/<pool>/apps/digibyte-data:/opt/blockchain/data
```

Replace the following values before deploying:

- `/mnt/<pool>/apps/digibyte-data` with the TrueNAS dataset path for the node.
- `CHANGE_THIS_TO_A_LONG_RANDOM_PASSWORD` with a long unique RPC password.
- `192.168.1.0/24` with the trusted LAN CIDR, if different.

The RPC username and password must match the values in the electrs configuration.
The RPC allow-list should be as narrow as possible. Do not publish port `14022`
to the Internet, and do not reuse the RPC password for another service.

Start the node and wait for it to finish its initial DigiByte synchronization.
Confirm that the container is healthy and that `txindex` has completed its
initial indexing before starting electrs.

## Step 2: Create the DigiByte electrs Custom App

Create a second TrueNAS Custom App using this Compose-compatible YAML. It joins
the existing `dgb-electrum-net` network created by the node app:

```yaml
name: electrs-digibyte

services:
  electrs:
    image: ghcr.io/digibyte-electrum/electrs:digibyte-support
    pull_policy: always
    container_name: dgb_electrs
    user: "568:568"
    restart: unless-stopped
    stop_grace_period: 2m
    command:
      - --conf
      - /electrs.toml
    configs:
      - source: electrs_config
        target: /electrs.toml
        uid: "568"
        gid: "568"
        mode: 0440
    networks:
      - dgb-electrum-net
    ports:
      - "50011:50001"
    volumes:
      - type: bind
        source: /mnt/<pool>/apps/electrs-digibyte
        target: /data

configs:
  electrs_config:
    content: |
      network = "digibyte"
      auth = "dgb_electrum:CHANGE_THIS_TO_A_LONG_RANDOM_PASSWORD"
      daemon_rpc_addr = "digibyte:14022"
      daemon_p2p_addr = "digibyte:12024"
      electrum_rpc_addr = "0.0.0.0:50001"
      monitoring_addr = "0.0.0.0:4225"
      db_dir = "/data"
      log_filters = "INFO"

networks:
  dgb-electrum-net:
    external: true
    name: dgb-electrum-net
```

Replace both `<pool>` paths with the appropriate TrueNAS dataset paths and use
the same RPC username/password configured in the DigiByte node app. The electrs
dataset must be writable by UID/GID `568`, or the container permissions must be
adjusted to match the ownership policy of the chosen dataset.

After deployment, follow the electrs logs. The first startup builds the index
and can take a significant amount of time. Restarting the container does not
discard a completed index; it resumes from the data stored in `/data`.

## Running directly with Docker on Linux

The TrueNAS instructions above are only one way to deploy this project. The
same DigiByte node and electrs containers can run on Debian, Ubuntu, or another
Linux AMD64 host with Docker Compose. TrueNAS Apps simply provides a graphical
way to manage the same container definitions.

The node and electrs containers must share the Docker network
`dgb-electrum-net`. The node is reached internally as `digibyte`, so the
containers do not need to communicate through the host IP address. Only electrs
needs to publish an external port for wallet connections.

Save the following as `digibyte-node.yml`, replacing the data path and RPC
password before starting it:

```yaml
networks:
  dgb-electrum-net:
    driver: bridge
    name: dgb-electrum-net

services:
  digibyte:
    command: >-
      digibyted -server=1 -listen=1 -txindex=1 -dbcache=256 -maxmempool=64
      -par=0 -rpcuser=dgb_electrum
      -rpcpassword=CHANGE_THIS_TO_A_LONG_RANDOM_PASSWORD
      -rpcbind=0.0.0.0 -rpcallowip=127.0.0.1/32
      -rpcallowip=192.168.1.0/24 -rpcallowip=172.16.0.0/12
      -rpcport=14022 -maxconnections=32
    container_name: dgb_full_node
    deploy:
      resources:
        limits:
          cpus: '6.0'
          memory: 8G
        reservations:
          memory: 4096M
    image: blocknetdx/digibyte:latest
    networks:
      dgb-electrum-net:
        aliases:
          - digibyte
    ports:
      - '14022:14022'
      - '12024:12024'
    restart: unless-stopped
    volumes:
      - /srv/digibyte/data:/opt/blockchain/data
```

The node's RPC port does not need to be published to the host because electrs
uses the Docker network. If host-side RPC access is required for administration,
publish `14022:14022` only to a trusted interface or restrict it with firewall
rules. Never expose the RPC port publicly.

Create the node and electrs applications from their respective Compose files:

```bash
docker compose -f digibyte-node.yml up -d
docker compose -f electrs-docker-compose.yml up -d
```

The electrs Compose file can use the same service definition shown in the
TrueNAS section, with these Linux-specific volume values:

```yaml
volumes:
  - /srv/electrs-digibyte/data:/data
```

Its network declaration must remain external because the node Compose project
created the shared network:

```yaml
networks:
  dgb-electrum-net:
    external: true
    name: dgb-electrum-net
```

The electrs configuration remains:

```toml
auth = "dgb_electrum:CHANGE_THIS_TO_A_LONG_RANDOM_PASSWORD"
daemon_rpc_addr = "digibyte:14022"
daemon_p2p_addr = "digibyte:12024"
electrum_rpc_addr = "0.0.0.0:50001"
```

The RPC username and password must be identical in both containers. Start the
DigiByte node first and wait for its blockchain and `txindex` to synchronize;
then start electrs and allow it to build its persistent index. The wallet
connects to the Linux host at `<host-address>:50011:t`, just as it does with a
TrueNAS deployment.

## Connecting Electrum-DigiByte

Configure the wallet to connect to the TrueNAS host running electrs:

```text
<truenas-host>:50011:t
```

Use **Connect only to a single server** while testing. The wallet should report
the electrs server height and eventually show synchronized status. Wallet header
verification remains enabled: electrs accelerates transaction queries, but the
wallet still validates the headers it receives according to DigiByte consensus.

## Updating and troubleshooting

- Update the DigiByte node first, allow it to resynchronize, then update electrs.
- Keep the node and electrs datasets persistent; deleting the electrs dataset
  forces the index to be rebuilt.
- If electrs cannot connect, check that both apps are attached to
  `dgb-electrum-net`, that the node service alias is `digibyte`, and that the
  RPC credentials match exactly.
- If electrs reports that the node is still catching up, wait for the node's
  initial blockchain and `txindex` synchronization.
- If the wallet cannot connect, verify that TrueNAS publishes host port `50011`
  and that the firewall permits it from the wallet's network.
- Keep RPC port `14022` restricted to the node network and trusted LAN hosts.

## Upstream project

This repository is based on the original Rust electrs project by Romanz. See
the upstream documentation for general architecture, Rust development, database
schema, monitoring, and contribution information. DigiByte-specific consensus,
network, image, and TrueNAS deployment behavior is documented here.

## License and logo

See the repository license and the upstream project for licensing details. The
README uses `logo/logo-digibyte.svg`, a DigiByte adaptation of the upstream
electrs mark. Its central DigiByte symbol is derived from the official
[DigiByte logos repository](https://github.com/DigiByte-Core/digibyte-logos).
The original upstream artwork remains available as `logo/logo.svg`.
