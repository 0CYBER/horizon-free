GROUP = 'item modify component patches'


def cases(cmd, setup):
    target = 'entity @s weapon.mainhand'
    selected = '/data get entity @s SelectedItem'

    def modify(patch, expect_result=1):
        return cmd(f'/item modify {target} {{type:"minecraft:set_components",components:{patch}}}',
                   expect_result=expect_result)

    return [
        *setup('/clear @s'),
        cmd(f'/item replace {target} with minecraft:mud_brick_stairs[break_sound="minecraft:block.stone.break",custom_data={{kept:1}}]'),
        modify('{"minecraft:break_sound":"entity.item.break"}'),
        cmd(f'{selected}.components', expect_result=1),
        cmd(f'/item replace {target} with minecraft:bush[custom_data={{temporary:1}}]'),
        cmd(f'/execute if items {target} *[minecraft:use_effects]', expect_result=1),
        modify('{"!minecraft:use_effects":{},"!minecraft:custom_data":{}}'),
        cmd(f'{selected}.components', expect_result=1),
        cmd(f'/execute if items {target} *[!minecraft:use_effects]', expect_result=1),
        cmd(f'/item replace {target} with minecraft:compass[custom_data={{kept:1}}] 64'),
        modify('{"minecraft:max_stack_size":16,"minecraft:rarity":"rare"}'),
        cmd(f'{selected}.count', expect_result=64),
        cmd(f'{selected}.components', expect_result=1),
        cmd(f'/item replace {target} with minecraft:compass 8'),
        modify('{"minecraft:max_stack_size":16b}'),
        cmd(f'{selected}.components."minecraft:max_stack_size"', expect_result=16),
        modify('{"minecraft:max_stack_size":100}', expect_result=None),
        cmd(f'{selected}.components."minecraft:max_stack_size"', expect_result=16),
        cmd(f'/item modify {target} {{type:"minecraft:sequence",functions:['
            '{type:"minecraft:set_components",components:{max_stack_size:24b}}]}',
            expect_result=1),
        cmd(f'{selected}.components."minecraft:max_stack_size"', expect_result=24),
        cmd(f'/item replace {target} with minecraft:chest[custom_data={{kept:1}}]'),
        modify('{"minecraft:container":[{slot:0,item:{id:"ender_pearl",count:17}}]}'),
        cmd(f'{selected}.components', expect_result=1),
        cmd(f'/item replace {target} with minecraft:bundle[custom_data={{kept:1}}]'),
        modify('{"minecraft:bundle_contents":[{id:"ender_pearl",count:17}]}'),
        cmd(f'{selected}.components', expect_result=1),
        modify('{bundle_contents:[{id:"compass",count:2,components:{max_stack_size:1}}]}'),
        cmd(f'{selected}.components', expect_result=1),
        modify('{bundle_contents:[{id:"compass",components:{max_stack_size:100}}]}',
               expect_result=None),
        cmd(f'{selected}.components', expect_result=1),
        modify('{bundle_contents:[{id:"compass",components:{max_stack_size:1b}}]}'),
        cmd(f'{selected}.components."minecraft:bundle_contents"[0].components."minecraft:max_stack_size"',
            expect_result=1),
        # validateStrict checks the chest template's count. The compass is
        # checked when the chest template is materialized as a live stack.
        modify('{bundle_contents:[{id:"chest",components:{container:[{slot:0,item:'
               '{id:"compass",count:2,components:{max_stack_size:1}}}]}}]}'),
        cmd(f'{selected}.components."minecraft:bundle_contents"[0].components'
            '."minecraft:container"[0].item.count', expect_result=2),
        cmd(f'/item replace {target} with minecraft:filled_map[custom_data={{kept:1}}]'),
        modify('{map_post_processing:"lock"}', expect_result=None),
        modify('{"!minecraft:map_post_processing":{}}', expect_result=None),
        cmd(f'{selected}.components', expect_result=1),
        cmd(f'/item replace {target} with minecraft:bush[custom_data={{kept:1}}]'),
        modify('{"minecraft:use_effects":{can_sprint:true}}'),
        cmd(f'{selected}.components', expect_result=2),
        modify('{"minecraft:use_effects":{can_sprint:false}}'),
        cmd(f'{selected}.components', expect_result=1),
        *setup('/clear @s'),
    ]
