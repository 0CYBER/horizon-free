import json
import unittest
from tools import test_canyon_carver_parity as canyon


class CanyonCarverCorpusTests(unittest.TestCase):
    def test_heights_custom_widths_and_failures(self):
        config={'type':'minecraft:canyon','probability':0.01,'y':{'absolute':40},'vertical_rotation':0,'shape':{'width_smoothness':3,'distance_factor':1,'thickness':3,'horizontal_radius_factor':1,'vertical_radius_default_factor':1,'vertical_radius_center_factor':0,'y_scale':3}}
        fixtures=list(canyon.corpus({'canyon':json.dumps(config)}))
        self.assertEqual(fixtures,list(canyon.corpus({'canyon':json.dumps(config)})))
        self.assertEqual({f['context'][1] for f in fixtures},{16,128,384,4064})
        self.assertEqual({f['fail'] for f in fixtures},{-1,0,1,63})
        self.assertTrue(any(f['target']==[-2**31,2**31-1] for f in fixtures))
        definitions=[json.loads(f['definition']) for f in fixtures]
        shapes=[d['shape'] for d in definitions if isinstance(d,dict) and isinstance(d.get('shape'),dict)]
        self.assertTrue(any(s.get('width_smoothness')==2**31-1 for s in shapes))
        self.assertTrue(any(s.get('distance_factor')==0 for s in shapes))


if __name__=='__main__':unittest.main()
