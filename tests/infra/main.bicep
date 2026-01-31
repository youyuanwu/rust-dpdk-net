// Main deployment template - supports 1 or 2 VMs
var baseName = resourceGroup().name
var location = resourceGroup().location

@description('SSH public key for VM access')
param sshPublicKey string

@description('VM1 size')
@allowed([
  'Standard_D2s_v5'
  'Standard_D4s_v5'
  'Standard_D8s_v5'
  'Standard_D16s_v5'
  'Standard_D32s_v5'
  'Standard_D48s_v5'
  'Standard_D64s_v5'
])
param vm1Size string = 'Standard_D2s_v5'

@description('VM2 size')
@allowed([
  'Standard_D2s_v5'
  'Standard_D4s_v5'
  'Standard_D8s_v5'
  'Standard_D16s_v5'
  'Standard_D32s_v5'
  'Standard_D48s_v5'
  'Standard_D64s_v5'
])
param vm2Size string = 'Standard_D2s_v5'

@description('Number of VMs to deploy (1 or 2)')
@allowed([1, 2])
param vmCount int = 1

@description('Number of NICs per VM (1 or 2). Use 2 for DPDK scenarios.')
@allowed([1, 2])
param nicsPerVm int = 1

@description('Enable auto-shutdown schedule')
param enableAutoShutdown bool = true

@description('Auto-shutdown time (24h format, e.g., 1900)')
param shutdownTime string = '1900'

// Naming
var vnetName = '${baseName}-vnet'
var nsgName = '${baseName}-nsg'
var vm1Name = vmCount == 1 ? baseName : '${baseName}-vm1'
var vm2Name = '${baseName}-vm2'

// Private IP address constants
// VM1: 10.0.0.4 (NIC1), 10.0.0.5 (NIC2)
// VM2: 10.0.0.5 or 10.0.0.6 (NIC1), 10.0.0.7 (NIC2)
var vm1Nic1Ip = '10.0.0.4'
var vm1Nic2Ip = '10.0.0.5'
var vm2Nic1Ip = nicsPerVm == 2 ? '10.0.0.6' : '10.0.0.5'
var vm2Nic2Ip = '10.0.0.7'

// Network Security Group
resource nsg 'Microsoft.Network/networkSecurityGroups@2024-07-01' = {
  name: nsgName
  location: location
  properties: {
    securityRules: []
  }
}

resource nsgRuleSsh 'Microsoft.Network/networkSecurityGroups/securityRules@2024-07-01' = {
  parent: nsg
  name: 'SSH'
  properties: {
    protocol: 'TCP'
    sourcePortRange: '*'
    destinationPortRange: '22'
    sourceAddressPrefix: '*'
    destinationAddressPrefix: '*'
    access: 'Allow'
    priority: 300
    direction: 'Inbound'
  }
}

resource nsgRuleVNet 'Microsoft.Network/networkSecurityGroups/securityRules@2024-07-01' = {
  parent: nsg
  name: 'AllowVNetInbound'
  properties: {
    protocol: '*'
    sourcePortRange: '*'
    destinationPortRange: '*'
    sourceAddressPrefix: 'VirtualNetwork'
    destinationAddressPrefix: 'VirtualNetwork'
    access: 'Allow'
    priority: 100
    direction: 'Inbound'
  }
}

resource nsgRuleDpdk 'Microsoft.Network/networkSecurityGroups/securityRules@2024-07-01' = {
  parent: nsg
  name: 'DPDK_TCP_Server'
  properties: {
    protocol: 'TCP'
    sourcePortRange: '*'
    destinationPortRange: '8080'
    sourceAddressPrefix: '*'
    destinationAddressPrefix: '*'
    access: 'Allow'
    priority: 310
    direction: 'Inbound'
  }
}

// Virtual Network
resource vnet 'Microsoft.Network/virtualNetworks@2024-07-01' = {
  name: vnetName
  location: location
  properties: {
    addressSpace: { addressPrefixes: ['10.0.0.0/16'] }
    subnets: [
      {
        name: 'default'
        properties: {
          addressPrefix: '10.0.0.0/24'
          networkSecurityGroup: { id: nsg.id }
        }
      }
    ]
  }
}

resource subnet 'Microsoft.Network/virtualNetworks/subnets@2024-07-01' existing = {
  parent: vnet
  name: 'default'
}

// Public IPs - VM1 always gets a public IP for SSH access
resource publicIp1 'Microsoft.Network/publicIPAddresses@2024-07-01' = {
  name: '${vm1Name}-pip'
  location: location
  sku: { name: 'Standard', tier: 'Regional' }
  zones: ['1']
  properties: {
    publicIPAddressVersion: 'IPv4'
    publicIPAllocationMethod: 'Static'
    idleTimeoutInMinutes: 4
  }
}

// Second public IP for VM1's second NIC (DPDK)
resource publicIp1Nic2 'Microsoft.Network/publicIPAddresses@2024-07-01' = if (nicsPerVm == 2) {
  name: '${vm1Name}-pip2'
  location: location
  sku: { name: 'Standard', tier: 'Regional' }
  zones: ['1']
  properties: {
    publicIPAddressVersion: 'IPv4'
    publicIPAllocationMethod: 'Static'
    idleTimeoutInMinutes: 4
  }
}

// VM2 public IP (optional - only if deploying 2 VMs)
resource publicIp2 'Microsoft.Network/publicIPAddresses@2024-07-01' = if (vmCount == 2) {
  name: '${vm2Name}-pip'
  location: location
  sku: { name: 'Standard', tier: 'Regional' }
  zones: ['1']
  properties: {
    publicIPAddressVersion: 'IPv4'
    publicIPAllocationMethod: 'Static'
    idleTimeoutInMinutes: 4
  }
}

// VM1
module vm1 'modules/vm.bicep' = {
  name: 'deploy-${vm1Name}'
  params: {
    vmName: vm1Name
    location: location
    vmSize: vm1Size
    sshPublicKey: sshPublicKey
    subnetId: subnet.id
    nsgId: nsg.id
    privateIpAddress: vm1Nic1Ip
    publicIpId: publicIp1.id
    nicCount: nicsPerVm
    privateIpAddress2: nicsPerVm == 2 ? vm1Nic2Ip : ''
    publicIpId2: nicsPerVm == 2 ? publicIp1Nic2.id : ''
  }
}

// VM2 (optional)
module vm2 'modules/vm.bicep' = if (vmCount == 2) {
  name: 'deploy-${vm2Name}'
  params: {
    vmName: vm2Name
    location: location
    vmSize: vm2Size
    sshPublicKey: sshPublicKey
    subnetId: subnet.id
    nsgId: nsg.id
    privateIpAddress: vm2Nic1Ip
    publicIpId: publicIp2.id
    nicCount: nicsPerVm
    privateIpAddress2: nicsPerVm == 2 ? vm2Nic2Ip : ''
    publicIpId2: ''  // No public IP for VM2's second NIC
  }
}

// Auto-shutdown schedules
resource shutdownSchedule1 'microsoft.devtestlab/schedules@2018-09-15' = if (enableAutoShutdown) {
  name: 'shutdown-computevm-${vm1Name}'
  location: location
  properties: {
    status: 'Enabled'
    taskType: 'ComputeVmShutdownTask'
    dailyRecurrence: { time: shutdownTime }
    timeZoneId: 'UTC'
    notificationSettings: { status: 'Disabled', timeInMinutes: 30, notificationLocale: 'en' }
    targetResourceId: vm1.outputs.vmId
  }
}

resource shutdownSchedule2 'microsoft.devtestlab/schedules@2018-09-15' = if (enableAutoShutdown && vmCount == 2) {
  name: 'shutdown-computevm-${vm2Name}'
  location: location
  properties: {
    status: 'Enabled'
    taskType: 'ComputeVmShutdownTask'
    dailyRecurrence: { time: shutdownTime }
    timeZoneId: 'UTC'
    notificationSettings: { status: 'Disabled', timeInMinutes: 30, notificationLocale: 'en' }
    #disable-next-line BCP318
    targetResourceId: vm2.outputs.vmId
  }
}

// Outputs
output vm1Name string = vm1.outputs.vmName
output vm1PrivateIp string = vm1.outputs.privateIp
output vm1PrivateIp2 string = nicsPerVm == 2 ? vm1Nic2Ip : ''
output vm1PublicIp string = publicIp1.properties.ipAddress
#disable-next-line BCP318
output vm1PublicIp2 string = nicsPerVm == 2 ? publicIp1Nic2.properties.ipAddress : ''
#disable-next-line BCP318
output vm2Name string = vmCount == 2 ? vm2.outputs.vmName : ''
#disable-next-line BCP318
output vm2PrivateIp string = vmCount == 2 ? vm2.outputs.privateIp : ''
output vm2PrivateIp2 string = vmCount == 2 && nicsPerVm == 2 ? vm2Nic2Ip : ''
#disable-next-line BCP318
output vm2PublicIp string = vmCount == 2 ? publicIp2.properties.ipAddress : ''
output sshCommand string = 'ssh azureuser@${publicIp1.properties.ipAddress}'
output sshToVm2FromVm1 string = vmCount == 2 ? 'ssh ${vm2Nic1Ip}' : ''
