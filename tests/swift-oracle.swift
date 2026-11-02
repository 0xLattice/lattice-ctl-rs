import Foundation
import Crypto
import Multikey
import Lattice
import cashew
struct Input:Decodable {let seed:UInt8;let fee:UInt64;let nonce:String;let chain:String;let kind:Int;let signature:String}
func pair(_ seed: UInt8) throws -> (String,String,String) {
 let privateKey=try Curve25519.Signing.PrivateKey(rawRepresentation:Data(repeating:seed,count:32))
 let publicKey=Multikey(keyType:.ed25519,keyBytes:privateKey.publicKey.rawRepresentation).hexEncoded
 return (privateKey.rawRepresentation.hexString,publicKey,CryptoUtils.createAddress(from:publicKey))
}
let rows=try JSONDecoder().decode([Input].self,from:Data(contentsOf:URL(fileURLWithPath:CommandLine.arguments[1])))
var result:[[String:String]]=[]
let to=try pair(42).2
for row in rows {
 let (privateKey,publicKey,address)=try pair(row.seed)
 var accounts:[AccountAction]=[];var general:[Action]=[];var deposits:[DepositAction]=[];var receipts:[ReceiptAction]=[];var withdrawals:[WithdrawalAction]=[]
 switch row.kind {
 case 0:accounts=[AccountAction(owner:address,delta:-23),AccountAction(owner:to,delta:23)]
 case 1:accounts=[AccountAction(owner:address,delta:-23)];deposits=[DepositAction(nonce:17,demander:address,amountDemanded:11,amountDeposited:23)]
 case 2:receipts=[ReceiptAction(withdrawer:address,nonce:17,demander:to,amountDemanded:11,directory:"Alpha")]
 case 3:accounts=[AccountAction(owner:address,delta:23)];withdrawals=[WithdrawalAction(withdrawer:address,nonce:17,demander:to,amountDemanded:11,amountWithdrawn:23)]
 default:general=[Action(key:"child/Alpha",oldValue:"old@state",newValue:to+"@"+address)]
 }
 if row.fee>0 {accounts.append(AccountAction(owner:address,delta:-Int64(row.fee)))}
 let body=TransactionBody(accountActions:accounts,actions:general,depositActions:deposits,receiptActions:receipts,withdrawalActions:withdrawals,signers:[address],nonce:UInt64(row.nonce)!,chainPath:row.chain.components(separatedBy:"/"))
 let header=try HeaderImpl(node:body)
 let signature=TransactionSigning.sign(bodyHeader:header,privateKeyHex:privateKey)!
 let transaction=Transaction(signatures:[publicKey:signature],body:header)
 result.append(["address":address,"publicKey":publicKey,"bodyCID":header.rawCID,"signature":signature,"rustSignatureAccepted":String(TransactionSigning.verify(bodyHeader:header,signature:row.signature,publicKeyHex:publicKey)),"preimage":TransactionSigning.preimage(bodyHeader:header)!,"transactionCID":try HeaderImpl(node:transaction).rawCID])
}
let encoder=JSONEncoder();encoder.outputFormatting=[.sortedKeys];print(String(decoding:try encoder.encode(result),as:UTF8.self))
