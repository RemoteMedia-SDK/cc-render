"""Declared bottle interactions using the project's existing interact protocol."""


def add_interactions(scene):
    shelf = scene['props'][0]['object']
    shelf['zones'] = [{'id':'approach','type':'point','center':shelf['approach'],'radius':.25}]
    shelf['interactions'] = [{'id':'face_outwards','phases':[
        {'id':'turn','prompt':'A person stands under the running shower, facing away from the wall.',
         'stand_at':{'zone':'approach','facing_deg':90},'timeout_s':12}]}]
    for prop in scene['props'][1:]:
        bottle = prop['object']
        name = bottle['id']
        local = [a-b for a,b in zip(bottle['position'], shelf['position'])]
        shelf['zones'].extend([
            {'id':name+'_return', 'type':'point', 'center':local, 'radius':.033},
            {'id':name+'_slot', 'type':'support', 'center':[local[0],0.,local[2]],
             'half_extents':[.065,.015,.065]}])
        bottle['zones'] = [
            {'id':'outlet', 'type':'working_point', 'center':[0.,.107,0.]},
            {'id':'approach', 'type':'point', 'center':bottle['approach'], 'radius':.25},
            {'id':'body', 'type':'segment', 'a':[0.,-.065,0.], 'b':[0.,.065,0.],
             'radius':.033, 'grip':'power'}]
        bottle['interactions'] = [
            {'id':'pick_up', 'phases':[
                {'id':'reach', 'prompt':'A person stands upright and reaches up with their right hand to take a bottle from a high shelf near head height.',
                 'stand_at':{'zone':'approach'}, 'relations':[
                     {'kind':'touch', 'joints':['RightHand'], 'zone':'body',
                      'window_s':[0,9], 'min_frames':3, 'tolerance_m':.08}],
                 'transition':{'kind':'attach', 'anchors':[{'joint':'RightHand','zone':'body'}]},
                 'timeout_s':10}]},
            {'id':'pour_into_hand', 'phases':[
                {'id':'pour', 'prompt':'A person pours from a bottle in their right hand into their open left palm.',
                 'dispense':{'outlet':'outlet','receiver':'LeftHand'}, 'timeout_s':15}]},
            {'id':'set_down', 'phases':[
                {'id':'return', 'prompt':'A person reaches forward with their right hand and places the bottle upright on the shelf in front of them, then lowers their empty hand.',
                 'stand_at':{'zone':'shower_caddy/approach'},
                 'relations':[{'kind':'touch','joints':['RightHand'],
                               'zone':'shower_caddy/'+name+'_return','window_s':[0,9],
                               'min_frames':3,'tolerance_m':.08}],
                 'transition':{'kind':'release','onto':'shower_caddy/'+name+'_slot'},
                 'timeout_s':10}]}]
