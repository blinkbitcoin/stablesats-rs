is_ci=sys.argv[1] == "ci"

local_resource(
  name='init-onchain',
  labels = ['dev-setup'],
  cmd='vendor/galoy-quickstart/bin/init-onchain.sh',
  resource_deps = [
    "bitcoind",
    "bria",
  ]
)
local_resource(
  name='init-lightning',
  labels = ['dev-setup'],
  cmd='vendor/galoy-quickstart/bin/init-lightning.sh',
  resource_deps = [
    "lnd1",
    "lnd-outside-1",
  ]
)

local_resource(
  name='setup-stablesats-db',
  labels = ['dev-setup'],
  cmd='make setup-db',
  resource_deps = [
    "stablesats-pg",
  ]
)

stablesats_serve_cmd = './target/debug/stablesats -c stablesats-dev.yml run'
if is_ci:
  stablesats_serve_cmd = './target/debug/stablesats -c stablesats-dev.yml run'

local_resource(
  name='stablesats-dev',
  labels = ['stablesats'],
  cmd='cargo build --bin stablesats',
  serve_cmd=stablesats_serve_cmd,
  links = [
      link("http://localhost:3325", "price-server"),
  ],
  resource_deps = [
      "init-onchain",
      "init-lightning",
      "setup-stablesats-db",
  ],
)

# if is_ci:
#   local_resource(
#     name='bats',
#     labels = ['dev-setup'],
#     cmd='bats test/bats',
#     resource_deps = [
#       "blink-kyc",
#     ],
#     allow_parallel = False
#   )

docker_compose(['vendor/galoy-quickstart/docker-compose.yml', 'docker-compose.yml', 'docker-compose.override.yml'])
galoy_services = ["apollo-router", "galoy", "trigger", "redis", "mongodb", "mongodb-migrate", "price", "price-history", "price-history-migrate", "price-history-pg", "svix", "svix-pg", "stablesats"]
auth_services = ["oathkeeper", "kratos", "kratos-pg", "hydra", "hydra-pg", "hydra-migrate"]
bitcoin_services = ["bitcoind", "bitcoind-signer", "lnd1", "lnd-outside-1", "bria", "bria-pg", "fulcrum"]
stablesats_services = ["stablesats-pg"]

for service in galoy_services:
    dc_resource(service, labels = ["galoy"])
for service in auth_services:
    dc_resource(service, labels = ["auth"])
for service in bitcoin_services:
    dc_resource(service, labels = ["bitcoin"])
for service in stablesats_services:
    dc_resource(service, labels = ["stablesats"])
# for service in blink_kyc:
#     dc_resource(service, labels = ["blink-kyc"])

dc_resource('otel-agent', labels = ["otel"])
dc_resource('quickstart-test', labels = ['quickstart'], auto_init=False)
