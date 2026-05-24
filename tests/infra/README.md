# Azure VM Deployment

## Prerequisites

- Azure CLI installed and logged in (`az login`)
- SSH key pair (`~/.ssh/id_ed25519.pub`)

## Quick Start

```sh
rg=tenant-test
az group create --name $rg --location westus2
```

## Deploy Single VM

By default the SSH (22) and DPDK test (8080) rules are open to the
internet (`['*']`). Lock them down to your current public IP by
passing `allowedSourceAddressPrefixes`:

```sh
MYIP=$(curl -s https://api.ipify.org)
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_ed25519.pub)" \
               nicsPerVm=1 \
               vm1Size=Standard_D8s_v5 \
               allowedSourceAddressPrefixes="['${MYIP}/32']"
```

Multiple CIDRs (home + office, etc.):

```sh
--parameters allowedSourceAddressPrefixes="['1.2.3.4/32','203.0.113.0/24']"
```

Intra-VNet traffic (10.0.0.0/16) is always allowed via a separate
`AllowVNetInbound` rule, so VM-to-VM communication is unaffected.
If your IP changes, just re-run the deployment with the new value —
only the NSG rule will update.

## Deploy ARM64 VM (Cobalt 100)

Pass an ARM size (`Standard_D{2,4,8,16,32,48,64,96}ps_v6`) as `vm1Size` /
`vm2Size`. The template auto-selects the `server-arm64` Ubuntu image
when an ARM size is chosen — no other changes needed.

```sh
--parameters vm1Size=Standard_D8ps_v6
```

## Deploy Single VM with 2 NICs (for DPDK)

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_ed25519.pub)" \
               nicsPerVm=2
```

## Deploy 2 VMs (Private Network Communication)

Both VMs are in the same subnet and can communicate via private IPs.

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_ed25519.pub)" \
               vmCount=2
```

## Deploy 2 VMs with 2 NICs Each

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_ed25519.pub)" \
               vmCount=2 \
               nicsPerVm=2 \
               vm1Size=Standard_D2s_v5 \
               vm2Size=Standard_D8s_v5
```

## Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `sshPublicKey` | (required) | SSH public key for VM access |
| `vmCount` | 1 | Number of VMs (1 or 2) |
| `nicsPerVm` | 1 | NICs per VM (1 or 2, use 2 for DPDK) |
| `vm1Size` | Standard_D2s_v5 | VM1 size |
| `vm2Size` | Standard_D2s_v5 | VM2 size |
| `enableAutoShutdown` | true | Auto-shutdown at 19:00 UTC |
| `shutdownTime` | 1900 | Shutdown time (24h format) |
| `allowedSourceAddressPrefixes` | `['*']` | CIDR list allowed to reach SSH (22) and port 8080. Set to `['<your-ip>/32']` to lock down. |

## IP Allocation

| Config | VM1 NIC1 | VM1 NIC2 | VM2 NIC1 | VM2 NIC2 |
|--------|----------|----------|----------|----------|
| 1 VM, 1 NIC | 10.0.0.4 | - | - | - |
| 1 VM, 2 NICs | 10.0.0.4 | 10.0.0.5 | - | - |
| 2 VMs, 1 NIC | 10.0.0.4 | - | 10.0.0.5 | - |
| 2 VMs, 2 NICs | 10.0.0.4 | 10.0.0.5 | 10.0.0.6 | 10.0.0.7 |

## Connect to VMs

```sh
# Get deployment outputs
az deployment group show -g $rg -n main --query properties.outputs

# SSH to VM1
ssh azureuser@<vm1-public-ip>

# SSH to VM2 from VM1 (private network)
ssh 10.0.0.5  # or 10.0.0.6 if using 2 NICs per VM
```

## Start and stop vm
```sh
az vm deallocate --resource-group $rg --name "$rg-vm1" 
az vm deallocate --resource-group $rg --name "$rg-vm2"

az vm start --resource-group $rg --name "$rg-vm1"
az vm start --resource-group $rg --name "$rg-vm2"
```

## Cleanup

```sh
az group delete --name $rg --yes --no-wait
```

## MISC
```sh
az bicep lint --file ./tests/infra/main.bicep 
```