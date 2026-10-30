package dependency

type Client struct{}

func (*Client) Live()        {}
func (*Client) FromInit()    {}
func (*Client) FromClosure() {}
func (*Client) Dead()        {}

type API interface{ Live() }

func Wrapper()          { invoke(&Client{}) }
func invoke(client API) { client.Live() }
func Initialize()       { (&Client{}).FromInit() }
func FromClosure()      { (&Client{}).FromClosure() }

// Imported dependency code is not a root merely because it was loaded.
func unused() { (&Client{}).Dead() }
