# VM create

Follow instruction:
[Create VM with accelerated networking](https://learn.microsoft.com/en-us/azure/virtual-network/create-virtual-machine-accelerated-networking?tabs=portal)

```ps1

$resourceGroup = @{
    Name = "test-rg2"
    Location = "WestUS2"
}
New-AzResourceGroup @resourceGroup

$vnet1 = @{
    ResourceGroupName = "test-rg2"
    Location = "WestUS2"
    Name = "vnet-1"
    AddressPrefix = "10.0.0.0/16"
}
$virtualNetwork1 = New-AzVirtualNetwork @vnet1

$subConfig = @{
    Name = "subnet-1"
    AddressPrefix = "10.0.0.0/24"
    VirtualNetwork = $virtualNetwork1
}
$subnetConfig1 = Add-AzVirtualNetworkSubnetConfig @subConfig

$subBConfig = @{
    Name = "AzureBastionSubnet"
    AddressPrefix = "10.0.1.0/24"
    VirtualNetwork = $virtualNetwork1
}
$subnetConfig2 = Add-AzVirtualNetworkSubnetConfig @subBConfig

$virtualNetwork1 | Set-AzVirtualNetwork


# Create nic with accelerated networking
$vnetParams = @{
    ResourceGroupName = "test-rg2"
    Name = "vnet-1"
    }
$vnet = Get-AzVirtualNetwork @vnetParams

$nicParams = @{
    ResourceGroupName = "test-rg2"
    Name = "nic-1"
    Location = "WestUS2"
    SubnetId = $vnet.Subnets[0].Id
    EnableAcceleratedNetworking = $true
    }
$nic = New-AzNetworkInterface @nicParams

# create vm (no pwd use ssh)
$cred = Get-Credential
$vmConfigParams = @{
    VMName = "vm-1"
    VMSize = "Standard_D2s_v5"
    }
$vmConfig = New-AzVMConfig @vmConfigParams

$osParams = @{
    VM = $vmConfig
    ComputerName = "vm-1"
    Credential = $cred
    }
$vmConfig = Set-AzVMOperatingSystem @osParams -Linux -DisablePasswordAuthentication

$imageParams = @{
    VM = $vmConfig
    PublisherName = "Canonical"
    Offer = "ubuntu-24_04-lts"
    Skus = "server"
    Version = "latest"
    }
$vmConfig = Set-AzVMSourceImage @imageParams

# Get the network interface object
$nicParams = @{
    ResourceGroupName = "test-rg2"
    Name = "nic-1"
    }
$nic = Get-AzNetworkInterface @nicParams

$vmConfigParams = @{
    VM = $vmConfig
    Id = $nic.Id
    }
$vmConfig = Add-AzVMNetworkInterface @vmConfigParams

$vmParams = @{
    VM = $vmConfig
    ResourceGroupName = "test-rg2"
    Location = "westus2"
    SshKeyName = "ssh-key"
    }
New-AzVM @vmParams -GenerateSshKey

# public ip
$rg = "test-rg2"
$publicIp = New-AzPublicIpAddress -Name myPublicIP -ResourceGroupName "test-rg2" -AllocationMethod Static -Location westus2
$nic = Get-AzNetworkInterface -ResourceGroupName $rg -Name "nic-1"
# attach public ip.
$nic.IpConfigurations[0].PublicIpAddress = $publicIp
Set-AzNetworkInterface -NetworkInterface $nic

# create nsg
$location = "westus2" 
$nsgName = "myVM-nsg" 
$nsg = New-AzNetworkSecurityGroup -ResourceGroupName $rg -Location $location -Name $nsgName

# allow ssh
$nsg | Add-AzNetworkSecurityRuleConfig `
  -Name "Allow-SSH" `
  -Description "Allow SSH inbound" `
  -Access Allow `
  -Protocol Tcp `
  -Direction Inbound `
  -Priority 1000 `
  -SourceAddressPrefix "*" `
  -SourcePortRange "*" `
  -DestinationAddressPrefix "*" `
  -DestinationPortRange 22 
  
Set-AzNetworkSecurityGroup -NetworkSecurityGroup $nsg

# set nsg on nic
$nic = Get-AzNetworkInterface -ResourceGroupName $rg -Name "nic-1" 
$nic.NetworkSecurityGroup = $nsg 
Set-AzNetworkInterface -NetworkInterface $nic
```