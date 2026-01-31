# Azure VM Deployment

## Prerequisites

- Azure CLI installed and logged in (`az login`)
- SSH key pair (`~/.ssh/id_rsa.pub`)

## Quick Start

```sh
rg=tenant-test
az group create --name $rg --location westus2
```

## Deploy Single VM

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_rsa.pub)"
```

## Deploy Single VM with 2 NICs (for DPDK)

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_rsa.pub)" \
               nicsPerVm=2
```

## Deploy 2 VMs (Private Network Communication)

Both VMs are in the same subnet and can communicate via private IPs.

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_rsa.pub)" \
               vmCount=2
```

## Deploy 2 VMs with 2 NICs Each

```sh
az deployment group create \
  --resource-group $rg \
  --template-file tests/infra/main.bicep \
  --parameters sshPublicKey="$(cat ~/.ssh/id_rsa.pub)" \
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