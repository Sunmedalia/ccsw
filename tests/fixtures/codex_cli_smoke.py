"""Optional real Codex CLI smoke test using a local mock upstream, no API credentials.
Run from repository root after cargo build:
  SMOKE_FORMAT=anthropic python3 tests/fixtures/codex_cli_smoke.py
Formats: anthropic, openai-chat, openai-responses. Set CCSW_CODEX_BIN if needed.
For Docker without user namespaces, set CCSW_SMOKE_SANDBOX=danger-full-access
only inside the isolated test container.
Only reads/writes generated files in a temporary home and executes fixed test commands.
"""
import http.server,threading,tempfile,pathlib,subprocess,json,os,socket
ccsw=str(pathlib.Path('target/debug/ccsw').resolve())
with tempfile.TemporaryDirectory(prefix='ccsw-cli-smoke-') as root:
 p=pathlib.Path(root);(p/'codex').mkdir();(p/'work').mkdir();(p/'work/example.txt').write_text('before\n')
 seen=[]
 kind=os.environ.get('SMOKE_FORMAT','openai-responses')
 class Handler(http.server.BaseHTTPRequestHandler):
  def log_message(self,*args):pass
  def do_POST(self):
   body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
   tools={}
   for t in body.get('tools',[]):
    t=t.get('function',t)
    if t.get('name'):tools[t['name']]=dict(t,parameters=t.get('parameters',t.get('input_schema',{})))
   step=len(seen);seen.append({'step':step,'tools':list(tools),'outputs':[v for v in body.get('input',[]) if v.get('type') in ['function_call_output','custom_tool_call_output']]})
   tool=next((name for name in ['exec_command','shell_command','shell'] if name in tools),None)
   if step<3 and tool:
    command=['cat example.txt',"printf 'after\\n' > example.txt","test \"$(cat example.txt)\" = after"][step]
    props=tools[tool].get('parameters',{}).get('properties',{})
    args={('cmd' if 'cmd' in props else 'command'):command}
    if 'command' in props and props['command'].get('type')=='array':args['command']=['sh','-c',command]
    if 'workdir' in props:args['workdir']=str(p/'work')
    item={'type':'function_call','id':f'fc_{step}','call_id':f'call_{step}','name':tool,'arguments':json.dumps(args),'status':'completed'}
   else:item={'type':'message','id':'msg_done','role':'assistant','status':'completed','content':[{'type':'output_text','text':'Smoke test completed.','annotations':[]}]}
   response={'id':f'resp_{step}','object':'response','created_at':0,'model':'deepseek-v4-flash','status':'completed','output':[item],'usage':{'input_tokens':1,'output_tokens':1,'total_tokens':2}}
   events=[{'type':'response.created','response':dict(response,status='in_progress',output=[])},{'type':'response.output_item.added','output_index':0,'item':dict(item,status='in_progress')},{'type':'response.output_item.done','output_index':0,'item':item},{'type':'response.completed','response':response}]
   if kind=='anthropic':
    block={'type':'tool_use','id':item['call_id'],'name':item['name'],'input':{}} if item['type']=='function_call' else {'type':'text','text':''}
    delta={'type':'input_json_delta','partial_json':item['arguments']} if item['type']=='function_call' else {'type':'text_delta','text':'Smoke test completed.'}
    events=[{'type':'message_start','message':{'id':f'msg_{step}','model':'deepseek-v4-flash','role':'assistant','content':[],'usage':{'input_tokens':1,'output_tokens':0}}},{'type':'content_block_start','index':0,'content_block':block},{'type':'content_block_delta','index':0,'delta':delta},{'type':'content_block_stop','index':0},{'type':'message_delta','delta':{'stop_reason':'tool_use' if item['type']=='function_call' else 'end_turn'},'usage':{'output_tokens':1}},{'type':'message_stop'}]
   elif kind=='openai-chat':
    delta={'tool_calls':[{'index':0,'id':item['call_id'],'type':'function','function':{'name':item['name'],'arguments':item['arguments']}}]} if item['type']=='function_call' else {'content':'Smoke test completed.'}
    events=[{'type':'chat','id':f'chat_{step}','model':'deepseek-v4-flash','choices':[{'index':0,'delta':delta,'finish_reason':None}]},{'type':'chat','id':f'chat_{step}','model':'deepseek-v4-flash','choices':[{'index':0,'delta':{},'finish_reason':'tool_calls' if item['type']=='function_call' else 'stop'}],'usage':{'prompt_tokens':1,'completion_tokens':1}}]
   out=''.join('event: '+e['type']+'\ndata: '+json.dumps(dict(e,sequence_number=i))+'\n\n' for i,e in enumerate(events)).encode()
   if kind=='openai-chat':out+=b'data: [DONE]\n\n'
   self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(out)));self.end_headers();self.wfile.write(out)
 server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
 (p/'config.toml').write_text(f"version=3\n[profiles.local]\nname='Local smoke'\nbase_url='http://127.0.0.1:{server.server_port}/v1'\napi_format='{kind}'\ndefault_model='deepseek-v4-flash'\n")
 env=dict(os.environ,HOME=root,USERPROFILE=root,CODEX_HOME=str(p/'codex'),CCSW_CONFIG=str(p/'config.toml'),XDG_STATE_HOME=str(p/'state'),XDG_CACHE_HOME=str(p/'cache'))
 for k in ['OPENAI_API_KEY','CODEX_ACCESS_TOKEN','CODEX_AUTH']:env.pop(k,None)
 sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
 def run(args):
  result=subprocess.run([ccsw]+args,env=env,capture_output=True,text=True,timeout=20)
  if result.returncode:raise RuntimeError(result.stderr)
 try:
  run(['proxy','start','--listen',f'127.0.0.1:{port}']);run(['codex','apply','--profile','local'])
  result=subprocess.run([os.environ.get('CCSW_CODEX_BIN','codex'),'exec','--skip-git-repo-check','--sandbox',os.environ.get('CCSW_SMOKE_SANDBOX','workspace-write'),'-c','approval_policy="never"','-c','web_search="disabled"','-C',str(p/'work'),'Read example.txt, replace before with after, and verify.'],env=env,capture_output=True,text=True,timeout=45)
  print('cli_exit',result.returncode,'requests',len(seen),'file',repr((p/'work/example.txt').read_text()))
  print('tools',seen[0]['tools'] if seen else [])
  print('output_count',[len(s['outputs']) for s in seen])
  assert 'Model metadata for' not in result.stderr, 'Custom model metadata warning still present'
  print('metadata_warning', False)
  assert result.returncode==0 and (p/'work/example.txt').read_text()=='after\n' and len(seen)==4, 'CLI tool workflow failed'
 finally:
  run(['proxy','stop']);server.shutdown()
