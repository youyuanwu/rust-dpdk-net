// Reusable VM module
@description('VM name')
param vmName string

@description('Location for resources')
param location string

@description('VM size')
param vmSize string

@description('SSH public key')
param sshPublicKey string

@description('Subnet resource ID')
param subnetId string

@description('NSG resource ID')
param nsgId string

@description('Private IP address for the VM')
param privateIpAddress string

@description('Public IP resource ID (optional - leave empty for private-only)')
param publicIpId string = ''

@description('Enable accelerated networking')
param enableAcceleratedNetworking bool = true

@description('Availability zone')
param zone string = '1'

@description('Number of NICs (1 or 2)')
@minValue(1)
@maxValue(2)
param nicCount int = 1

@description('Second private IP (used when nicCount=2)')
param privateIpAddress2 string = ''

@description('Second public IP resource ID (used when nicCount=2)')
param publicIpId2 string = ''

// Auto-select Ubuntu image SKU: ARM (Dpsv6 Cobalt 100, etc.) gets
// the 'server-arm64' image; x86 sizes get the standard 'server' image.
var isArm = contains(vmSize, 'ps_v') || contains(vmSize, 'pls_v')
var imageSku = isArm ? 'server-arm64' : 'server'

// Primary NIC
resource nic1 'Microsoft.Network/networkInterfaces@2024-07-01' = {
  name: '${vmName}-nic1'
  location: location
  properties: {
    ipConfigurations: [
      {
        name: 'ipconfig1'
        properties: {
          privateIPAddress: privateIpAddress
          privateIPAllocationMethod: 'Static'
          publicIPAddress: !empty(publicIpId) ? { id: publicIpId } : null
          subnet: { id: subnetId }
          primary: true
          privateIPAddressVersion: 'IPv4'
        }
      }
    ]
    enableAcceleratedNetworking: enableAcceleratedNetworking
    enableIPForwarding: false
    networkSecurityGroup: { id: nsgId }
  }
}

// Secondary NIC (optional, for DPDK scenarios)
resource nic2 'Microsoft.Network/networkInterfaces@2024-07-01' = if (nicCount == 2) {
  name: '${vmName}-nic2'
  location: location
  properties: {
    ipConfigurations: [
      {
        name: 'ipconfig1'
        properties: {
          privateIPAddress: privateIpAddress2
          privateIPAllocationMethod: 'Static'
          publicIPAddress: !empty(publicIpId2) ? { id: publicIpId2 } : null
          subnet: { id: subnetId }
          primary: true
          privateIPAddressVersion: 'IPv4'
        }
      }
    ]
    enableAcceleratedNetworking: enableAcceleratedNetworking
    enableIPForwarding: false
    networkSecurityGroup: { id: nsgId }
  }
}

resource vm 'Microsoft.Compute/virtualMachines@2024-11-01' = {
  name: vmName
  location: location
  zones: [zone]
  properties: {
    hardwareProfile: { vmSize: vmSize }
    additionalCapabilities: { hibernationEnabled: false }
    storageProfile: {
      imageReference: {
        publisher: 'canonical'
        offer: 'ubuntu-24_04-lts'
        sku: imageSku
        version: 'latest'
      }
      osDisk: {
        osType: 'Linux'
        name: '${vmName}_OsDisk'
        createOption: 'FromImage'
        caching: 'ReadWrite'
        managedDisk: { storageAccountType: 'Premium_LRS' }
        deleteOption: 'Delete'
        diskSizeGB: 30
      }
      diskControllerType: 'SCSI'
    }
    osProfile: {
      computerName: vmName
      #disable-next-line adminusername-should-not-be-literal
      adminUsername: 'azureuser'
      linuxConfiguration: {
        disablePasswordAuthentication: true
        ssh: {
          publicKeys: [
            {
              path: '/home/azureuser/.ssh/authorized_keys'
              keyData: sshPublicKey
            }
          ]
        }
        provisionVMAgent: true
        patchSettings: {
          patchMode: 'ImageDefault'
          assessmentMode: 'ImageDefault'
        }
      }
      allowExtensionOperations: true
    }
    securityProfile: {
      uefiSettings: {
        secureBootEnabled: true
        vTpmEnabled: true
      }
      securityType: 'TrustedLaunch'
    }
    networkProfile: {
      networkInterfaces: nicCount == 2 ? [
        { id: nic1.id, properties: { deleteOption: 'Detach', primary: true } }
        { id: nic2.id, properties: { deleteOption: 'Detach', primary: false } }
      ] : [
        { id: nic1.id, properties: { deleteOption: 'Detach', primary: true } }
      ]
    }
    diagnosticsProfile: { bootDiagnostics: { enabled: true } }
  }
}

output vmId string = vm.id
output vmName string = vm.name
output privateIp string = privateIpAddress
output nic1Id string = nic1.id
output nic2Id string = nicCount == 2 ? nic2.id : ''
