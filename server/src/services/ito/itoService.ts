import type { ConnectRouter } from '@connectrpc/connect'
import {
  ItoService as ItoServiceDesc,
  TranscribeStreamRequest,
} from '../../generated/ito_pb.js'
import type { HandlerContext } from '@connectrpc/connect'
import { transcribeStreamV2Handler } from './transcribeStreamV2Handler.js'

// Export the service implementation as a function that takes a ConnectRouter
export default (router: ConnectRouter) => {
  router.service(ItoServiceDesc, {
    async transcribeStreamV2(
      requests: AsyncIterable<TranscribeStreamRequest>,
      context: HandlerContext,
    ) {
      return transcribeStreamV2Handler.process(requests, context)
    },
  })
}
