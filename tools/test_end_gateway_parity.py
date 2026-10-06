#!/usr/bin/env python3
"""Compare native End gateway geometry and conditional exit configuration."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    get = lambda name: facts['defaults']['minecraft:' + name]
    def definition(exact=False, exit=None):
        value = {'type': 'end_gateway', 'exact': exact}
        if exit is not None: value['exit'] = exit
        return value
    def case(value=None, **extra):
        result = {'definition': json.dumps(definition() if value is None else value), 'mode': 'feature',
                  'origin': [0,64,0], 'legacy': False, 'seed': 0, 'world_seed': 1, 'fail': -1, 'reject': 0,
                  'brightness': 15, 'fallback': get('stone'), 'states': [], 'repeats': 1, 'gateway_entity': 'gateway'}
        result.update(extra); return result
    # Every possible world-call failure around the 45 writes and entity update,
    # including the dirty callback after the exit has already changed.
    for exact, exit, entity, legacy, fail, reject in itertools.product(
            [False,True], [None,[0,64,0],[2147483647,-2147483648,-17]], ['none','other','gateway'],
            [False,True], [-1,*range(48)], [0,1,3]):
        yield case(definition(exact,exit),gateway_entity=entity,legacy=legacy,fail=fail,reject=reject,repeats=2)
    for origin, exact in itertools.product([[0,-64,0],[-17,319,-33],[2147483600,2147483600,-2147483600],[-2147483600,-2147483600,2147483600]],[False,True]):
        yield case(definition(exact,[-1,0,1]),origin=origin,repeats=3)
    # Wrapped betweenClosed boxes may be huge; stop after a bounded native prefix.
    for axis, coordinate, fail in itertools.product(range(3),[-2147483648,-2147483647,2147483646,2147483647],[0,1,5,17,43]):
        origin=[0,64,0];origin[axis]=coordinate
        yield case(definition(True,[0,0,0]),origin=origin,fail=fail)
    for value in [None,False,True,0,1,-1,0.5,'true','false',[],{}]:
        yield case({'type':'end_gateway','exact':value,'exit':[0,0,0]})
    yield case({'type':'end_gateway','exit':[0,0,0]})
    for exit in [None,False,True,0,'0',[],[1],[1,2],[1,2,3,4],{},[0,0,0],[1.9,-1.9,4294967297],['1',2,3],[None,2,3],[1e30,-1e30,1e-30],[2147483647,-2147483648,0]]:
        yield case({'type':'end_gateway','exact':True,'exit':exit})
    for legacy, seed, entity, fail, reject in itertools.product([False,True],[0,-1,123456789],['none','gateway'],[-1,0,22,23,24,47],[0,1,3]):
        placed={'feature':definition(True,[12,-64,-33]),'placement':[{'type':'count','count':2},{'type':'in_square'}]}
        yield case(placed,mode='placed',legacy=legacy,seed=seed,gateway_entity=entity,fail=fail,reject=reject)
        yield case({'type':'overlay','features':[placed,{'feature':definition(False,[0,319,0]),'placement':[]}]},legacy=legacy,seed=seed,gateway_entity=entity,fail=fail,reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-end-gateway-parity', __file__,
        'Native EndGatewayFeature required exact and optional exit codecs, 3x5x3 inclusive box traversal, air/bedrock/gateway geometry, update flags and conditional gateway block-entity exit updates. Native actual TheEndGatewayBlockEntity.setExitPosition mutates exit/exact fields before a traced dirty notification; missing/other entities are no-ops. All world-call failure boundaries, rejected writes, repeated updates, both random modes, placement composition, extreme coordinates, bounded overflow prefixes and malformed/numeric codecs are compared. Fixture block entities are controlled independently of writes and their fields are compared after success or partial failure; automatic entity lifecycle, world dirty propagation, gateway teleportation/search and full End generation are not simulated. Rust bounds pathological box work to 2^20 visits.')
